pub struct Tokenizer<'a> {
    escaping: bool,
    in_quotes: bool,
    completed: bool,
    slice: &'a str,
}

impl<'a> Tokenizer<'a> {
    pub fn new(string: &'a str) -> Tokenizer<'a> {
        let slice = skip_insignificant_characters(string);

        Tokenizer {
            escaping: false,
            in_quotes: false,
            completed: false,
            slice: slice,
        }
    }
}

//static PUNCTUATORS: &'static [&str] = &[ &"!", &"$", &"(", &")", &"...", &":", &"=", &"@", &"[", &"]", &"{", &"|", &"}" ];

static TOKEN_BREAKER: &'static [char] = &['{', '}', '(', ')', '[', ']', ':', '=', '$', '.'];

impl<'a> Iterator for Tokenizer<'a> {
    type Item = &'a str;
    fn next(&mut self) -> Option<Self::Item> {
        if self.completed {
            return None;
        }

        let mut iterator = self.slice.char_indices();
        let mut start = 0usize;
        loop {
            match iterator.next() {
                Some((pos, c))
                    if !self.in_quotes
                        && (TOKEN_BREAKER.contains(&c) || is_insignificant_character(c)) =>
                {
                    let processing_insignificant_character = is_insignificant_character(c);

                    if pos > start {
                        let result = &self.slice[start..pos];
                        self.slice = &self.slice[pos..];
                        return Some(result);
                    } else if c == '.' {
                        match iterator.next() {
                            Some((_, '.')) => match iterator.next() {
                                Some((_, '.')) => {
                                    self.slice = &self.slice[start + 3..];
                                    return Some("...");
                                }
                                _ => {
                                    self.slice = &self.slice[start + 2..];
                                    return Some("..");
                                }
                            },
                            _ => {
                                self.slice = &self.slice[start + 1..];
                                return Some(".");
                            }
                        }
                    } else if !processing_insignificant_character && pos == start {
                        let result = &self.slice[start..pos + 1];
                        self.slice = &self.slice[pos + 1..];
                        return Some(result);
                    } else {
                        start = pos + 1;
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
