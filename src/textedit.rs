//! Editable text with a caret and a selection, shared by the address bar and form fields.
//! Positions are character indices (not bytes).

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextEdit {
    pub text: String,
    /// Where the caret is.
    pub caret: usize,
    /// The other end of the selection (equal to `caret` when nothing is selected).
    pub anchor: usize,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl TextEdit {
    /// Caret at the end.
    pub fn new(text: &str) -> TextEdit {
        let n = text.chars().count();
        TextEdit { text: text.to_string(), caret: n, anchor: n }
    }

    /// Everything selected (clicking into the address bar).
    pub fn select_all(text: &str) -> TextEdit {
        let mut e = TextEdit::new(text);
        e.anchor = 0;
        e
    }

    pub fn len(&self) -> usize {
        self.text.chars().count()
    }

    fn byte(&self, i: usize) -> usize {
        self.text.char_indices().nth(i).map_or(self.text.len(), |(b, _)| b)
    }

    /// The selected range, start <= end.
    pub fn selection(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    pub fn has_selection(&self) -> bool {
        self.caret != self.anchor
    }

    pub fn selected_text(&self) -> String {
        let (a, b) = self.selection();
        self.text[self.byte(a)..self.byte(b)].to_string()
    }

    /// Replace the selection (if any) with `s` and put the caret after it.
    pub fn insert(&mut self, s: &str) {
        let (a, b) = self.selection();
        let (ba, bb) = (self.byte(a), self.byte(b));
        self.text.replace_range(ba..bb, s);
        self.caret = a + s.chars().count();
        self.anchor = self.caret;
    }

    pub fn backspace(&mut self, word: bool) {
        if !self.has_selection() {
            self.anchor = if word { self.word_left(self.caret) } else { self.caret.saturating_sub(1) };
        }
        self.insert("");
    }

    pub fn delete(&mut self, word: bool) {
        if !self.has_selection() {
            self.anchor = if word { self.word_right(self.caret) } else { (self.caret + 1).min(self.len()) };
        }
        self.insert("");
    }

    fn chars(&self) -> Vec<char> {
        self.text.chars().collect()
    }

    /// Start of the word before `i` (skipping spaces and punctuation first).
    fn word_left(&self, i: usize) -> usize {
        let c = self.chars();
        let mut i = i.min(c.len());
        while i > 0 && !is_word(c[i - 1]) {
            i -= 1;
        }
        while i > 0 && is_word(c[i - 1]) {
            i -= 1;
        }
        i
    }

    /// End of the word after `i`.
    fn word_right(&self, i: usize) -> usize {
        let c = self.chars();
        let mut i = i.min(c.len());
        while i < c.len() && !is_word(c[i]) {
            i += 1;
        }
        while i < c.len() && is_word(c[i]) {
            i += 1;
        }
        i
    }

    /// Move the caret to `to`; with `extend`, keep the selection's other end where it is.
    pub fn move_to(&mut self, to: usize, extend: bool) {
        self.caret = to.min(self.len());
        if !extend {
            self.anchor = self.caret;
        }
    }

    pub fn left(&mut self, word: bool, extend: bool) {
        // Without Shift, Left on a selection goes to its start.
        if !extend && self.has_selection() && !word {
            let a = self.selection().0;
            return self.move_to(a, false);
        }
        let to = if word { self.word_left(self.caret) } else { self.caret.saturating_sub(1) };
        self.move_to(to, extend);
    }

    pub fn right(&mut self, word: bool, extend: bool) {
        if !extend && self.has_selection() && !word {
            let b = self.selection().1;
            return self.move_to(b, false);
        }
        let to = if word { self.word_right(self.caret) } else { self.caret + 1 };
        self.move_to(to, extend);
    }

    /// Home/End within the current line (the whole text for a single-line field).
    pub fn home(&mut self, extend: bool) {
        let c = self.chars();
        let mut i = self.caret.min(c.len());
        while i > 0 && c[i - 1] != '\n' {
            i -= 1;
        }
        self.move_to(i, extend);
    }

    pub fn end(&mut self, extend: bool) {
        let c = self.chars();
        let mut i = self.caret.min(c.len());
        while i < c.len() && c[i] != '\n' {
            i += 1;
        }
        self.move_to(i, extend);
    }

    /// Up/Down in a multi-line field: same column on the previous/next line (by characters).
    pub fn vertical(&mut self, down: bool, extend: bool) {
        let c = self.chars();
        let caret = self.caret.min(c.len());
        let line_start = |mut i: usize| {
            while i > 0 && c[i - 1] != '\n' {
                i -= 1;
            }
            i
        };
        let line_end = |mut i: usize| {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
            i
        };
        let start = line_start(caret);
        let col = caret - start;
        let to = if down {
            let end = line_end(caret);
            if end >= c.len() {
                c.len()
            } else {
                let next = end + 1;
                (next + col).min(line_end(next))
            }
        } else if start == 0 {
            0
        } else {
            let prev = line_start(start - 1);
            (prev + col).min(start - 1)
        };
        self.move_to(to, extend);
    }

    pub fn select_all_text(&mut self) {
        self.anchor = 0;
        self.caret = self.len();
    }

    /// Select the word around character `i` (double-click).
    pub fn select_word_at(&mut self, i: usize) {
        let c = self.chars();
        let i = i.min(c.len());
        let (mut a, mut b) = (i, i);
        while a > 0 && is_word(c[a - 1]) {
            a -= 1;
        }
        while b < c.len() && is_word(c[b]) {
            b += 1;
        }
        self.anchor = a;
        self.caret = b;
    }

    /// Keep the caret valid after the text changed underneath (a script set the value).
    pub fn sync(&mut self, text: &str) {
        if self.text != text {
            self.text = text.to_string();
            let n = self.len();
            self.caret = self.caret.min(n);
            self.anchor = self.anchor.min(n);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editing() {
        let mut e = TextEdit::new("hello world");
        e.left(true, false);
        assert_eq!(e.caret, 6);
        e.insert("big ");
        assert_eq!(e.text, "hello big world");
        e.home(true);
        assert_eq!(e.selected_text(), "hello big ");
        e.insert("");
        assert_eq!(e.text, "world");
        e.end(false);
        e.backspace(true);
        assert_eq!(e.text, "");
        let mut e = TextEdit::new("héllo");
        e.left(false, false);
        e.backspace(false);
        assert_eq!(e.text, "hélo");
        let mut e = TextEdit::new("ab\ncdef\ng");
        e.move_to(5, false); // "cd|ef"
        e.vertical(false, false);
        assert_eq!(e.caret, 2); // end of "ab"
        e.vertical(true, false);
        assert_eq!(e.caret, 5);
        e.vertical(true, false);
        assert_eq!(e.caret, 9); // "g|"
        let mut e = TextEdit::new("one two");
        e.select_word_at(5);
        assert_eq!(e.selected_text(), "two");
    }
}
