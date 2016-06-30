/// Model represents the global states needed in FZF.
/// It will also define how the states will be shown on the terminal


use std::sync::{Arc, RwLock};
use item::{Item, MatchedItem, MatchedRange};
use ncurses::*;
use std::cmp::{min, max};
use std::cell::RefCell;
use std::collections::HashSet;
use orderedvec::OrderedVec;
use curses::*;
use query::Query;
use util::eventbox::EventBox;
use event::Event;
use std::mem;

// The whole screen is:
//
//                  +---------------------------------------|
//                  | | |                                   | 5
//                  | | |               ^                   | 4
//   current cursor |>| |               |                   | 3
//                  | | |      lines    |                   | 2 cursor
//         selected | |>|--------------------------------   | 1
//                  | | |                                   | 0
//                  +---------------------------------------+
//          spinner |/| | (matched/total) (per%) [selected] |
//                  +---------------------------------------+
//                  | prompt>  query string                 |
//                  +---------------------------------------+
//

pub struct Model {
    eb: Arc<EventBox<Event>>,
    pub query: Query,

    num_matched: u64,
    num_total: u64,
    pub items: Arc<RwLock<Vec<Item>>>, // all items
    selected_indics: HashSet<usize>,
    pub matched_items: RefCell<OrderedVec<MatchedItem>>,
    processed_percentage: u64,

    item_cursor: usize, // the index of matched item currently highlighted.
    line_cursor: usize, // line No.
    hscroll_offset: usize,

    max_y: i32,
    max_x: i32,
    width: usize,
    height: usize,

    tabstop: usize,
    curses: Curses,
}

impl Model {
    pub fn new(eb: Arc<EventBox<Event>>, curses: Curses) -> Self {
        let (max_y, max_x) = curses.get_maxyx();

        Model {
            eb: eb,
            query: Query::new(),
            num_matched: 0,
            num_total: 0,
            items: Arc::new(RwLock::new(Vec::new())),
            selected_indics: HashSet::new(),
            matched_items: RefCell::new(OrderedVec::new()),
            processed_percentage: 100,
            item_cursor: 0,
            line_cursor: 0,
            hscroll_offset: 0,
            max_y: max_y,
            max_x: max_x,
            width: (max_x - 2) as usize,
            height: (max_y - 2) as usize,
            tabstop: 8,
            curses: curses,
        }
    }

    pub fn output(&self) {
        let mut selected = self.selected_indics.iter().collect::<Vec<&usize>>();
        selected.sort();
        let items = self.items.read().unwrap();
        for index in selected {
            println!("{}", items[*index].text);
        }
    }

    pub fn update_process_info(&mut self, matched: u64, total: u64, processed: u64) {
        self.num_matched = matched;
        self.num_total = total;
        self.processed_percentage = (processed+1)*100/(total+1);
    }

    pub fn push_item(&mut self, item: MatchedItem) {
        self.matched_items.borrow_mut().push(item);
    }

    pub fn clear_items(&mut self) {
        self.matched_items.borrow_mut().clear();
    }

    pub fn print_query(&self) {
        // > query
        mv(self.max_y-1, 0);
        addstr("> ");
        addstr(&self.query.get_query());
        mv(self.max_y-1, (self.query.pos+2) as i32);
    }

    pub fn print_info(&self) {
        mv(self.max_y-2, 0);
        addstr(format!("  {}/{}{} ", self.num_matched, self.num_total,
                       if self.processed_percentage == 100 {"".to_string()} else {format!("({}%)", self.processed_percentage)},
                       ).as_str());
    }

    pub fn print_items(&self) {
        let mut matched_items = self.matched_items.borrow_mut();
        let item_start_pos = self.item_cursor - self.line_cursor;

        for i in 0..self.height {
            if let Some(matched) = matched_items.get(item_start_pos + i) {
                mv((self.height - i - 1) as i32, 0);

                let is_current_line = i == self.line_cursor;
                let label = if is_current_line {">"} else {" "};
                self.curses.cprint(label, COLOR_CURSOR, true);
                self.print_item(matched, is_current_line);
            } else {
                break;
            }
        }
    }

    fn print_item(&self, matched: &MatchedItem, is_current: bool) {
        let items = self.items.read().unwrap();
        let ref item = items[matched.index];

        let is_selected = self.selected_indics.contains(&matched.index);

        if is_selected {
            self.curses.cprint(">", COLOR_SELECTED, true);
        } else {
            self.curses.cprint(" ", if is_current {COLOR_CURRENT} else {COLOR_NORMAL}, false);
        }

        match matched.matched_range {
            Some(MatchedRange::Chars(ref matched_indics)) => {
                let matched_end_pos = if matched_indics.len() > 0 {
                    matched_indics[matched_indics.len()-1]
                } else {
                    0
                };

                let (text, mut idx) = reshape_string(&item.text.chars().collect::<Vec<char>>(),
                                                     (self.max_x-3) as usize,
                                                     self.hscroll_offset,
                                                     matched_end_pos);
                let mut matched_indics_iter = matched_indics.iter().peekable();

                // skip indics
                while let Some(&&index) = matched_indics_iter.peek() {
                    if idx > index {
                        let _ = matched_indics_iter.next();
                    } else {
                        break;
                    }
                }

                for &ch in text.iter() {
                    match matched_indics_iter.peek() {
                        Some(&&index) if idx == index => {
                            self.print_char(ch, COLOR_MATCHED, is_current);
                            let _ = matched_indics_iter.next();
                        }
                        Some(_) | None => {
                            self.print_char(ch, if is_current {COLOR_CURRENT} else {COLOR_NORMAL}, is_current)
                        }
                    }
                    idx += 1;
                }
            }
            Some(MatchedRange::Range(_, _)) => {
                // pass
            }
            None => {
                // pass
            }
        }
    }

    fn print_char(&self, ch: char, color: i16, is_bold: bool) {
        if ch != '\t' {
            self.curses.caddch(ch, color, is_bold);
        } else {
            // handle tabstop
            let mut y = 0;
            let mut x = 0;
            getyx(stdscr, &mut y, &mut x);
            let rest = (self.tabstop as i32) - (x-2)%(self.tabstop as i32);
            for i in 0..rest {
                self.curses.caddch(' ', color, is_bold);
            }
        }
    }

    pub fn refresh(&self) {
        refresh();
    }

    pub fn display(&self) {
        erase();
        self.print_items();
        self.print_info();
        self.print_query();
    }

    // the terminal resizes, so we need to recalculate the margins.
    pub fn resize(&mut self) {
        clear();
        endwin();
        refresh();
        let mut max_y = 0;
        let mut max_x = 0;
        getmaxyx(stdscr, &mut max_y, &mut max_x);
        self.max_y = max_y;
        self.max_x = max_x;
    }

    //============================================================================
    // Actions

    pub fn act_add_char(&mut self, ch: char) {
        let changed = self.query.add_char(ch);
        if changed {
            self.eb.set(Event::EvQueryChange, Box::new(self.query.get_query()));
        }
    }

    pub fn act_backward_char(&mut self) {
        let _ = self.query.backward_char();
    }

    pub fn act_backward_delete_char(&mut self) {
        let changed = self.query.backward_delete_char();
        if changed {
            self.eb.set(Event::EvQueryChange, Box::new(self.query.get_query()));
        }
    }

    pub fn act_backward_kill_word(&mut self) {
        let changed = self.query.backward_kill_word();
        if changed {
            self.eb.set(Event::EvQueryChange, Box::new(self.query.get_query()));
        }
    }

    pub fn act_backward_word(&mut self) {
        let _ = self.query.backward_word();
    }

    pub fn act_beginning_of_line(&mut self) {
        let _ = self.query.beginning_of_line();
    }

    pub fn act_delete_char(&mut self) {
        let changed = self.query.delete_char();
        if changed {
            self.eb.set(Event::EvQueryChange, Box::new(self.query.get_query()));
        }
    }

    pub fn act_deselect_all(&mut self) {
        self.selected_indics.clear();
    }

    pub fn act_end_of_line(&mut self) {
        let _ = self.query.end_of_line();
    }

    pub fn act_forward_char(&mut self) {
        let _ = self.query.forward_char();
    }

    pub fn act_forward_word(&mut self) {
        let _ = self.query.forward_word();
    }

    pub fn act_kill_line(&mut self) {
        let changed = self.query.kill_line();
        if changed {
            self.eb.set(Event::EvQueryChange, Box::new(self.query.get_query()));
        }
    }

    pub fn act_kill_word(&mut self) {
        let changed = self.query.kill_word();
        if changed {
            self.eb.set(Event::EvQueryChange, Box::new(self.query.get_query()));
        }
    }

    pub fn act_line_discard(&mut self) {
        let changed = self.query.line_discard();
        if changed {
            self.eb.set(Event::EvQueryChange, Box::new(self.query.get_query()));
        }
    }

    pub fn act_select_all(&mut self) {
        let mut matched_items = self.matched_items.borrow_mut();
        for i in 0..matched_items.len() {
            self.selected_indics.insert(i);
        }
    }

    pub fn act_toggle_all(&mut self) {
        let mut matched_items = self.matched_items.borrow_mut();
        let selected = mem::replace(&mut self.selected_indics, HashSet::new());
        for i in 0..matched_items.len() {
            if !selected.contains(&i) {
                self.selected_indics.insert(i);
            }
        }
    }

    pub fn act_toggle(&mut self, selected: Option<bool>) {
        let mut matched_items = self.matched_items.borrow_mut();
        let matched = matched_items.get(self.item_cursor);
        if matched == None {
            return;
        }

        let index = matched.unwrap().index;
        match selected {
            Some(true) => {
                let _ = self.selected_indics.insert(index);
            }
            Some(false) => {
                let _ = self.selected_indics.remove(&index);
            }
            None => {
                if self.selected_indics.contains(&index) {
                    let _ = self.selected_indics.remove(&index);
                } else {
                    let _ = self.selected_indics.insert(index);
                }
            }
        }
    }

    pub fn get_num_selected(&self) -> usize {
        self.selected_indics.len()
    }

    pub fn act_move_line_cursor(&mut self, diff: i32) {
        let total_item = self.matched_items.borrow().len() as i32;

        let y = self.line_cursor as i32 + diff;
        self.line_cursor = if diff > 0 {
            let tmp = min(min(y, (self.height as i32) -1), total_item-1);
            if tmp < 0 {0} else {tmp as usize}
        } else {
            max(0, y) as usize
        };


        let item_y = self.item_cursor as i32 + diff;
        self.item_cursor = if diff > 0 {
            let tmp = min(item_y, total_item-1);
            if tmp < 0 {0} else {tmp as usize}
        } else {
            max(0, item_y) as usize
        }
    }

    pub fn act_move_page(&mut self, pages: i32) {
        let lines = (self.height as i32) * pages;
        self.act_move_line_cursor(lines);
    }
}

//==============================================================================
// helper functions

// wide character will take two unit
fn display_width(text: &[char]) -> usize {
    text.iter()
        .map(|c| {if c.len_utf8() > 1 {2} else {1}})
        .fold(0, |acc, n| acc + n)
}


// calculate from left to right, stop when the max_x exceeds
fn left_fixed(text: &[char], max_x: usize) -> usize {
    if max_x <= 0 {
        return 0;
    }

    let mut w = 0;
    for (idx, &c) in text.iter().enumerate() {
        w += if c.len_utf8() > 1 {2} else {1};
        if w > max_x {
            return idx-1;
        }
    }
    return text.len()-1;
}

fn right_fixed(text: &[char], max_x: usize) -> usize {
    if max_x <= 0 {
        return text.len()-1;
    }

    let mut w = 0;
    for (idx, &c) in text.iter().enumerate().rev() {
        w += if c.len_utf8() > 1 {2} else {1};
        if w > max_x {
            return idx+1;
        }
    }
    return 0;

}

// return a string and its left position in original string
// matched_end_pos is char-wise
fn reshape_string(text: &Vec<char>,
                  container_width: usize,
                  text_start_pos: usize,
                  matched_end_pos: usize) -> (Vec<char>, usize) {
    let full_width = display_width(&text[text_start_pos..]);

    if full_width <= container_width {
        return (text[text_start_pos..].iter().map(|x| *x).collect(), text_start_pos);
    }

    let mut ret = Vec::new();
    let mut ret_pos = 0;

    // trim right, so that 'String' -> 'Str..'
    let right_pos = 1 + max(matched_end_pos, text_start_pos + left_fixed(&text[text_start_pos..], container_width-2));
    let mut left_pos = text_start_pos + right_fixed(&text[text_start_pos..right_pos], container_width-2);
    ret_pos = left_pos;

    if left_pos > text_start_pos {
        left_pos = text_start_pos + right_fixed(&text[text_start_pos..right_pos], container_width-4);
        ret.push('.'); ret.push('.');
        ret_pos = left_pos - 2;
    }

    // so we should print [left_pos..(right_pos+1)]
    for ch in text[left_pos..right_pos].iter() {
        ret.push(*ch);
    }
    ret.push('.'); ret.push('.');
    (ret, ret_pos)
}

#[cfg(test)]
mod test {
    #[test]
    fn test_display_width() {
        assert_eq!(super::display_width(&"abcdefg".to_string().chars().collect::<Vec<char>>()), 7);
        assert_eq!(super::display_width(&"This is 中国".to_string().chars().collect::<Vec<char>>()), 12);
    }

    #[test]
    fn test_left_fixed() {
        assert_eq!(super::left_fixed(&"a中cdef".to_string().chars().collect::<Vec<char>>(), 5), 3);
        assert_eq!(super::left_fixed(&"a中".to_string().chars().collect::<Vec<char>>(), 5), 1);
        assert_eq!(super::left_fixed(&"a中".to_string().chars().collect::<Vec<char>>(), 0), 0);
    }

    #[test]
    fn test_right_fixed() {
        assert_eq!(super::right_fixed(&"a中cdef".to_string().chars().collect::<Vec<char>>(), 5), 2);
        assert_eq!(super::right_fixed(&"a中".to_string().chars().collect::<Vec<char>>(), 5), 0);
        assert_eq!(super::right_fixed(&"a中".to_string().chars().collect::<Vec<char>>(), 0), 1);
    }

    #[test]
    fn test_reshape_string() {
        assert_eq!(super::reshape_string(&"0123456789".to_string().chars().collect::<Vec<char>>(),
                                         6, 1, 7),
                   ("..67..".to_string().chars().collect::<Vec<char>>(), 4));

        assert_eq!(super::reshape_string(&"0123456789".to_string().chars().collect::<Vec<char>>(),
                                         12, 1, 7),
                   ("123456789".to_string().chars().collect::<Vec<char>>(), 1));

        assert_eq!(super::reshape_string(&"0123456789".to_string().chars().collect::<Vec<char>>(),
                                         6, 0, 6),
                   ("..56..".to_string().chars().collect::<Vec<char>>(), 3));

        assert_eq!(super::reshape_string(&"0123456789".to_string().chars().collect::<Vec<char>>(),
                                         8, 0, 4),
                   ("012345..".to_string().chars().collect::<Vec<char>>(), 0));

        assert_eq!(super::reshape_string(&"0123456789".to_string().chars().collect::<Vec<char>>(),
                                         10, 0, 4),
                   ("0123456789".to_string().chars().collect::<Vec<char>>(), 0));
    }



}
