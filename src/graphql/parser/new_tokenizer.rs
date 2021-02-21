use std::str::CharIndices;

pub struct NewTokenizer<'a> {
    escaping: bool,
    in_quotes: bool,
    completed: bool,
    slice: &'a str,
    start: usize,
    iterator: CharIndices<'a>,
    last_char: Option<(usize, char)>
}

impl<'a> NewTokenizer<'a> {
    pub fn new(string: &'a str) -> NewTokenizer<'a> {
        let slice = skip_insignificant_characters(string);

        NewTokenizer {
            escaping: false,
            in_quotes: false,
            completed: false,
            slice: slice,
            start: 0usize,
            iterator: slice.char_indices(),
            last_char: None
        }
    }
}

//static PUNCTUATORS: &'static [&str] = &[ &"!", &"$", &"(", &")", &"...", &":", &"=", &"@", &"[", &"]", &"{", &"|", &"}" ];

static TOKEN_BREAKER: &'static [char] = &['{', '}', '(', ')', '[', ']', ':', '=', '$', '.'];

impl<'a> Iterator for NewTokenizer<'a> {
    type Item = &'a str;
    fn next(&mut self) -> Option<Self::Item> {
        if self.completed {
            return None;
        }

        loop {
            let vvv = self.last_char.or_else(|| self.iterator.next());
            self.last_char = None;

            match vvv {
                Some((pos, c))
                    if !self.in_quotes
                        && (TOKEN_BREAKER.contains(&c) || is_insignificant_character(c)) =>
                {
                    let processing_insignificant_character = is_insignificant_character(c);

                    if pos > self.start {
                        let result = &self.slice[self.start..pos];
                        self.start = pos;
                        self.last_char = Some((pos, c));
                        return Some(result);
                    } else if c == '.' {
                        match self.iterator.next() {
                            Some((_, '.')) => match self.iterator.next() {
                                Some((_, '.')) => {
                                    self.start += 3;
                                    return Some("...");
                                }
                                _ => {
                                    self.start += 2;
                                    return Some("..");
                                }
                            },
                            _ => {
                                self.start += 1;
                                return Some(".");
                            }
                        }
                    } else if !processing_insignificant_character && pos == self.start {
                        let result = &self.slice[self.start..pos + 1];
                        self.start = pos + 1;
                        return Some(result);
                    } else {
                        self.start = pos + 1;
                        continue;
                    }
                }
                Some((_, '\\')) => {
                    self.escaping = true;
                }
                Some((_, '"')) if !self.escaping => {
                    self.escaping = false;
                    self.in_quotes = !self.in_quotes;
                }
                Some(_) => {
                    self.escaping = false;
                }
                None => {
                    self.completed = true;
                    return None;
                }
            }
        }
    }
}

fn is_insignificant_character(c: char) -> bool {
    c.is_whitespace() || c == ','
}

fn skip_insignificant_characters<'a>(string: &'a str) -> &'a str {
    let mut chars = string.char_indices();
    loop {
        match chars.next() {
            Some((_, c)) if is_insignificant_character(c) => {}
            Some((ix, _)) => return &string[ix..],
            None => return &"",
        }
    }
}
