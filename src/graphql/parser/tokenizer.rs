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

#[test]
fn tokenizer_processes_simple_query() {
    let tokenizer = Tokenizer::new("{ f1 { f2 }}");
    let tokens = tokenizer.collect::<Vec<_>>();

    assert_eq!(6, tokens.len());
    assert_eq!("{", tokens[0]);
    assert_eq!("f1", tokens[1]);
    assert_eq!("{", tokens[2]);
    assert_eq!("f2", tokens[3]);
    assert_eq!("}", tokens[4]);
    assert_eq!("}", tokens[5]);
}

#[test]
fn tokenizer_processes_parameters() {
    let tokenizer = Tokenizer::new("{ f1(p1: 1,                          p2: \"parm2\") { f2 }}");
    let tokens = tokenizer.collect::<Vec<_>>();

    assert_eq!(14, tokens.len());
    assert_eq!("{", tokens[0]);
    assert_eq!("f1", tokens[1]);
    assert_eq!("(", tokens[2]);
    assert_eq!("p1", tokens[3]);
    assert_eq!(":", tokens[4]);
    assert_eq!("1", tokens[5]);
    assert_eq!("p2", tokens[6]);
    assert_eq!(":", tokens[7]);
    assert_eq!("\"parm2\"", tokens[8]);
    assert_eq!(")", tokens[9]);
    assert_eq!("{", tokens[10]);
    assert_eq!("f2", tokens[11]);
    assert_eq!("}", tokens[12]);
    assert_eq!("}", tokens[13]);
}

#[test]
fn tokenizer_processes_fragments() {
    let tokenizer =
        Tokenizer::new("{ f1(p1: 1,                          p2: \"parm2\") { f2 ...frag }}");
    let tokens = tokenizer.collect::<Vec<_>>();

    assert_eq!(16, tokens.len());
    assert_eq!("{", tokens[0]);
    assert_eq!("f1", tokens[1]);
    assert_eq!("(", tokens[2]);
    assert_eq!("p1", tokens[3]);
    assert_eq!(":", tokens[4]);
    assert_eq!("1", tokens[5]);
    assert_eq!("p2", tokens[6]);
    assert_eq!(":", tokens[7]);
    assert_eq!("\"parm2\"", tokens[8]);
    assert_eq!(")", tokens[9]);
    assert_eq!("{", tokens[10]);
    assert_eq!("f2", tokens[11]);
    assert_eq!("...", tokens[12]);
    assert_eq!("frag", tokens[13]);
    assert_eq!("}", tokens[14]);
    assert_eq!("}", tokens[15]);
}
