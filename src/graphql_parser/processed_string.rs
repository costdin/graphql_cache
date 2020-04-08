use std::str::Chars;

pub struct ProcessedString<'a> {
    escaping: bool,
    in_quotes: bool,
    completed: bool,
    skipped_char: Option<char>,
    chars: Chars<'a>,
}

impl<'a> ProcessedString<'a> {
    pub fn new(string: &'a String) -> ProcessedString<'a> {
        let mut chars = string.chars();
        let skipped_char = skip_insignificant_characters(&mut chars);

        ProcessedString {
            escaping: false,
            in_quotes: false,
            completed: false,
            chars: chars,
            skipped_char: skipped_char,
        }
    }
}

static PUNCTUATORS: &'static [&str] = &[ &"!", &"$", &"(", &")", &"...", &":", &"=", &"@", &"[", &"]", &"{", &"|", &"}" ];

static TOKEN_BREAKER: &'static [char] = &[ '{', '}', '(', ')', '[', ']', ':', '=', '$', '.' ];

impl Iterator for ProcessedString<'_> {
    type Item = String;
    fn next(&mut self) -> Option<Self::Item> {
        if self.completed {
            return None;
        }

        let mut token = match self.skipped_char {
            Some('.') => {
                match self.chars.next() {
                    Some('.') => match self.chars.next() {
                        Some('.') => { self.skipped_char = None; return Some(String::from("...")); },
                        c         => { self.skipped_char = c; return Some(String::from("..")); },
                    },
                    c         => { self.skipped_char = c; return Some(String::from(".")); }
                }
            }
            Some(c) if TOKEN_BREAKER.contains(&c) => {
                self.skipped_char = None;
                return Some(c.to_string());
            },
            Some(c) if is_insignificant_character(c) => String::new(),
            Some('"') => {
                self.in_quotes = true;
                String::from("\"")
            },
            Some(c) => c.to_string(),
            _       => String::new()
        };

        self.skipped_char = None;

        loop {
            match self.chars.next() {
                Some(c) if !self.in_quotes && (TOKEN_BREAKER.contains(&c) || is_insignificant_character(c)) => {
                    if token.len() > 0 {
                        self.skipped_char = Some(c);
                        return Some(token);
                    } else if is_insignificant_character(c) {
                        continue;
                    } else if c == '.' {
                        match self.chars.next() {
                            Some('.') => match self.chars.next() {
                                Some('.') => { self.skipped_char = None; return Some(String::from("...")); },
                                c         => { self.skipped_char = c; return Some(String::from("..")); },
                            },
                            c         => { self.skipped_char = c; return Some(String::from(".")); }
                        }        
                    } else {
                        return Some(c.to_string());
                    }
                },
                Some('\\') => {
                    self.escaping = true;
                    token.push('\\');
                },
                Some('"') if !self.escaping => {
                    self.escaping = false;
                    self.in_quotes = !self.in_quotes;
                    token.push('"');
                },
                Some(c) => {
                    self.escaping = false;
                    token.push(c);
                },
                None => {
                    self.completed = true;
                    return if token.len() > 0 { Some(token) } else { None };
                }
            }
        }
    }
}

fn is_insignificant_character(c: char) -> bool {
    c.is_whitespace() || c == ','
}

fn skip_insignificant_characters(chars: &mut Chars) -> Option<char> {
    loop {
        match chars.next() {
            Some(c) if is_insignificant_character(c) => { },
            c   => return c
        }
    }
}
