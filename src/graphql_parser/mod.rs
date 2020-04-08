mod processed_string;
use processed_string::ProcessedString;

pub fn parse_query(query: String) -> Result<Document, Error> {

    let mut operations = Vec::<Operation>::new();
    let mut fragment_definitions = Vec::<FragmentDefinition>::new();
    let mut parser_state = ParserState { hierarchy: Vec::<String>::new() }; 
    let mut tokens = ProcessedString::new(&query);
    let mut query_shorthand = true;

    loop {
        match tokens.next() {
            Some(s) if s == "query" => { query_shorthand = false; operations.push(parse_operation(tokens.next(), query_shorthand, OperationType::Query, &mut tokens, &mut parser_state)?) },
            Some(s) if s == "mutation" => { query_shorthand = false; operations.push(parse_operation(tokens.next(), query_shorthand, OperationType::Mutation, &mut tokens, &mut parser_state)?) },
            Some(s) if s == "subscription" => { query_shorthand = false; operations.push(parse_operation(tokens.next(), query_shorthand, OperationType::Subscription, &mut tokens, &mut parser_state)?) },
            Some(s) if s == "fragment" => { fragment_definitions.push(parse_fragment_definition(&mut tokens, &mut parser_state)?) },
            Some(s) if s == "{" && !query_shorthand => return Err(Error { error: String::from("Operation type is required when not in shorthand mode") }),
            Some(s) if s == "{" => { query_shorthand = true; operations.push(parse_operation(Some(s), query_shorthand, OperationType::Query, &mut tokens, &mut parser_state)?) },
            Some(s) => return Err(Error::new(format!("invalid token \"{}\"", s))),
            None    if operations.len() > 0 => return Ok(Document { operations: operations, fragment_definitions: fragment_definitions }),
            None    => return Err(Error::new(String::from("Unexpected end of string")))
        };

        if query_shorthand && operations.len() > 1 {
            return Err(Error { error: String::from("Only one operation allowed in shorthand mode") });
        }
    }
}

fn parse_fragment_definition<'a, I>(tokens: &mut I, parser_state: &mut ParserState) -> Result<FragmentDefinition, Error> 
    where I: Iterator<Item = String> {

    let name = match tokens.next() {
        Some(name) if is_valid_name(&name) => name,
        Some(s) => return Err(Error::new(format!("invalid token {}", s))),
        None    => return Err(Error::new(String::from("Unexpected end of string")))
    };

    match tokens.next() {
        Some(on) if on == "on" => { },
        Some(s) => return Err(Error::new(format!("invalid token {}", s))),
        None    => return Err(Error::new(String::from("Unexpected end of string")))
    };

    let type_name = match tokens.next() {
        Some(type_name) if is_valid_name(&type_name) => type_name,
        Some(s) => return Err(Error::new(format!("invalid token {}", s))),
        None    => return Err(Error::new(String::from("Unexpected end of string")))
    };

    let fields = match tokens.next() {
        Some(t) if t == "{" => {
            parser_state.hierarchy.push(t);
            parse_fields(tokens, parser_state)?
        },
        Some(s) => return Err(Error::new(format!("invalid token {}", s))),
        None    => return Err(Error::new(String::from("Unexpected end of string")))
    };

    Ok(FragmentDefinition{ name: name, r#type: type_name, fields: fields })
}

fn parse_operation<'a, I>(current_token: Option<String>, query_shorthand: bool, operation_type: OperationType, tokens: &mut I, parser_state: &mut ParserState) -> Result<Operation, Error>
    where I: Iterator<Item = String> {

    let (next_token, operation_name, variables) = match current_token {
        Some(s) if s == "{" => (Some(s), None, Vec::<Variable>::new()),
        Some(_) if query_shorthand => return Err(Error { error: String::from("Operation name is not allowed in shorthand mode") }),
        Some(s) if is_valid_name(&s) => {
            match tokens.next() {
                Some(t) if t == "(" => {
                    parser_state.hierarchy.push(t);
                    let variables = parse_variables(tokens, parser_state)?;

                    (tokens.next(), Some(s), variables)        
                },
                Some(t) if t == "{" => (Some(t), Some(s), Vec::<Variable>::new()),
                Some(s) => return Err(Error::new(format!("invalid token {}", s))),
                None    => return Err(Error::new(String::from("Unexpected end of string")))        
            }
        },
        Some(s) => return Err(Error::new(format!("invalid token {}", s))),
        None    => return Err(Error::new(String::from("Unexpected end of string")))
    };

    match next_token {
        Some(t) if t == "{" => {
            parser_state.hierarchy.push(t);
            let fields = parse_fields(tokens, parser_state)?;

            return Ok(Operation{
                operation_type: operation_type,
                name: operation_name,
                fields: fields,
                variables: variables
            });
        },
        Some(t) if t == "}" => return Err(Error { error: String::from("Unmatched parenteses") }),
        None => return Err(Error::new(String::from("Unexpected end of string"))),
        Some(t) => return Err(Error::new(format!("invalid token {}", t)))
    }
}

fn parse_variables<'a, I>(tokens: &mut I, parser_state: &mut ParserState) -> Result<Vec<Variable>, Error> 
    where I: Iterator<Item = String> {

    let mut variables = Vec::<Variable>::new();

    let mut next_token = tokens.next();
    loop {
        match next_token {
            Some(s) if s == ")" && is_matching_close_parenteses(&s, parser_state.hierarchy.pop())
                => return Ok(variables),
            Some(s) if s == ")" => return Err(Error { error: String::from("Unmatched parenteses") }),
            Some(s) if s == "$" => {
                let name = match tokens.next() {
                    Some(n) if is_valid_name(&n) => n,
                    Some(n) => return Err(Error::new(format!("invalid variable name {}", n))),
                    None    => return Err(Error::new(String::from("Unexpected end of string")))
                };

                match tokens.next() {
                    Some(n) if n == ":" => { },
                    Some(n) => return Err(Error::new(format!("invalid token {}", n))),
                    None    => return Err(Error::new(String::from("Unexpected end of string")))
                };

                let variable_type = match tokens.next() {
                    Some(n) if is_valid_type(&n, tokens, parser_state) => n,
                    Some(n) => return Err(Error::new(format!("invalid type {}", n))),
                    None    => return Err(Error::new(String::from("Unexpected end of string")))
                };

                let default_value = match tokens.next() {
                    Some(n) if n == "=" => {
                        match tokens.next() {
                            Some(v) if is_valid_value(&v) => { next_token = tokens.next(); Some(ParameterValue::Scalar(v)) },
                            Some(n) => return Err(Error::new(format!("invalid variable value {}", n))),
                            None    => return Err(Error::new(String::from("Unexpected end of string")))
                        }
                    },
                    Some(n) => { next_token = Some(n); None },
                    None    => return Err(Error::new(String::from("Unexpected end of string")))
                };

                variables.push(Variable{ name: name, r#type: variable_type, default_value: default_value});
            },
            Some(s) => return Err(Error::new(format!("invalid token {}", s))),
            None    => return Err(Error::new(String::from("Unexpected end of string")))
        };
    }
}

fn parse_fields<'a, I>(tokens: &mut I, parser_state: &mut ParserState) -> Result<Vec<Field>, Error> 
    where I: Iterator<Item = String> {

    let mut fields = Vec::<Field>::new();
    let mut next_token = tokens.next();

    loop {
        let new_field = match next_token {
            Some(s) if s == "..." => {
                match tokens.next() {
                    Some(fragment_name) if is_valid_name(&fragment_name) => {
                        next_token = tokens.next();

                        Field::new_fragment(fragment_name)
                    },
                    Some(t) => return Err(Error::new(format!("invalid token {}", t))),
                    None    => return Err(Error::new(String::from("Unexpected end of string")))
                }
            },
            Some(candidate_name) if is_valid_name(&candidate_name) => {
                next_token = tokens.next();
                
                let (alias, name) = match next_token {
                    Some(s) if s == ":" =>
                        match tokens.next() {
                            Some(n) if is_valid_name(&n) => {
                                next_token = tokens.next();
                                (Some(candidate_name), n)
                            },
                            Some(s) => return Err(Error::new(format!("invalid token {}", s))),
                            None    => return Err(Error::new(String::from("Unexpected end of string")))
                        },  
                    None    => return Err(Error::new(String::from("Unexpected end of string"))),
                    _ => (None, candidate_name)
                }; 
        
                let parameters = match next_token {
                    Some(s) if s == "(" => {
                        parser_state.hierarchy.push(s);
                        let params = parse_parameters(tokens, parser_state)?;
                        next_token = tokens.next();
        
                        params
                    }
                    None    => return Err(Error::new(String::from("Unexpected end of string"))),
                    _ => Vec::<Parameter>::new()
                };
        
                let subfields = match next_token {
                    Some(s) if s == "{" => {
                        parser_state.hierarchy.push(s);
                        let flds = parse_fields(tokens, parser_state)?;
                        next_token = tokens.next();
        
                        flds
                    },
                    _ => Vec::<Field>::new()
                };

                Field::new_field(alias, name, parameters, subfields)
            },
            Some(s) => return Err(Error::new(format!("invalid token {}", s))),
            None    => return Err(Error::new(String::from("Unexpected end of string")))
        };

        fields.push(new_field);

        match next_token {
            Some(s) if s == "}" && is_matching_close_parenteses(&s, parser_state.hierarchy.pop()) 
                => return Ok(fields),
            Some(s) if s == "}" => return Err(Error { error: String::from("Unmatched parenteses") }),
            _ => { }
        };
    }
}

fn parse_parameters<'a, I>(tokens: &mut I, parser_state: &mut ParserState) -> Result<Vec<Parameter>, Error> 
    where I: Iterator<Item = String> {

    let mut parameters = Vec::<Parameter>::new();

    loop {
        let name = match tokens.next() {
            Some(s) if s == ")" && is_matching_close_parenteses(&s, parser_state.hierarchy.pop())
                => return Ok(parameters),
            Some(s) if s == ")" => return Err(Error { error: String::from("Unmatched parenteses") }),
            Some(s) if is_valid_name(&s) => s,
            Some(s) => return Err(Error::new(format!("invalid token {}", s))),
            None    => return Err(Error::new(String::from("Unexpected end of string")))
        };

        match tokens.next() {
            Some(s) if s == ":" => { }
            Some(s)   => return Err(Error::new(format!("invalid token {}", s))),
            None      => return Err(Error::new(String::from("Unexpected end of string")))
        };

        let value = match tokens.next() {
            Some(s) if s == "{" => {
                parser_state.hierarchy.push(s);
                parse_object(tokens, parser_state)?
            },
            Some(s) if s == "[" => {
                parser_state.hierarchy.push(s);
                parse_list(tokens, parser_state)?
            },
            Some(s) if s == "$" => {
                match tokens.next() {
                    Some(variable_name) if is_valid_name(&variable_name) => ParameterValue::Variable(variable_name),
                    Some(s)   => return Err(Error::new(format!("invalid token {}", s))),
                    None      => return Err(Error::new(String::from("Unexpected end of string")))
                }
            }
            Some(s) if is_valid_value(&s) => ParameterValue::Scalar(s),
            Some(s) => return Err(Error::new(format!("invalid token {}", s))),
            None    => return Err(Error::new(String::from("Unexpected end of string")))
        };

        parameters.push(Parameter{ name: name, value: value });
    }
}

fn parse_object<'a, I>(tokens: &mut I, parser_state: &mut ParserState) -> Result<ParameterValue, Error> 
    where I: Iterator<Item = String> {

    let mut fields = Vec::<ParameterField>::new();

    loop {
        let name = match tokens.next() {
            Some(s) if (s == "}" || s == ")") && is_matching_close_parenteses(&s, parser_state.hierarchy.pop()) => return Ok(ParameterValue::Object(fields)),
            Some(s) if s == "}" || s == ")" => return Err(Error { error: String::from("Unmatched parenteses") }),
            Some(s) if s == "]" && is_matching_close_parenteses(&s, parser_state.hierarchy.pop()) => return Ok(ParameterValue::Nil),
            Some(s) => s,
            None    => return Err(Error::new(String::from("Unexpected end of string")))
        };
    
        match tokens.next() {
            Some(s) if s == ":" => { }
            Some(s)   => return Err(Error::new(format!("invalid token {}", s))),
            None      => return Err(Error::new(String::from("Unexpected end of string")))
        };

        let value = match tokens.next() {
            Some(s) if s == "{" => {
                parser_state.hierarchy.push(s);
                parse_object(tokens, parser_state)?
            },
            Some(s) if s == "[" => {
                parser_state.hierarchy.push(s);
                parse_list(tokens, parser_state)?
            }
            Some(s) if is_valid_value(&s) => ParameterValue::Scalar(s),
            Some(s) => return Err(Error::new(format!("invalid token {}", s))),
            None    => return Err(Error::new(String::from("Unexpected end of string")))
        };

        fields.push(ParameterField{ name: name, value: value });
    }
}

fn parse_list<'a, I>(tokens: &mut I, parser_state: &mut ParserState) -> Result<ParameterValue, Error> 
    where I: Iterator<Item = String> {

    let mut objs = Vec::<ParameterValue>::new();

    loop {
        match tokens.next() {
            Some(s) if s == "{" => {
                parser_state.hierarchy.push(s);
                objs.push(parse_object(tokens, parser_state)?);
            }
            Some(s) if s == "]" && is_matching_close_parenteses(&s, parser_state.hierarchy.pop()) => return Ok(ParameterValue::List(objs)),
            Some(s) if is_valid_value(&s) => objs.push(ParameterValue::Scalar(s)),
            Some(s)   => return Err(Error::new(format!("invalid token {}", s))),
            None      => return Err(Error::new(String::from("Unexpected end of string")))
        }
    }
}

fn is_valid_name(string: &String) -> bool {
    let mut chars = string.chars();

    return match chars.next() {
        Some(c) if !c.is_alphabetic() => false,
        Some(_) => chars.all(|c| c.is_alphanumeric() || c == '_'),
        None    => false
    };
}

fn is_valid_value(string: &String) -> bool {
    let mut chars = string.chars();

    return match chars.next() {
        Some('"') => chars.last().unwrap_or(' ') == '"',
        Some(c) if c.is_alphanumeric() => chars.all(|c| c.is_alphanumeric()),
        _         => false
    };
}

fn is_valid_type<'a, I>(string: &String, _tokens: &mut I, _parser_state: &mut ParserState) -> bool
    where I: Iterator<Item = String> {

    is_valid_name(string)
}

fn is_matching_close_parenteses(close: &String, open_option: Option<String>) -> bool {
    match open_option {
        Some(open) => (close == "}" && open == "{") ||
                      (close == ")" && open == "(") ||
                      (close == "]" && open == "["),
        None => false
    }
}

struct ParserState {
    hierarchy: Vec<String>
}

#[derive(Debug)]
pub struct Variable {
    pub name: String,
    pub r#type: String,
    pub default_value: Option<ParameterValue>
}

#[derive(Debug)]
pub struct Document {
    pub operations: Vec<Operation>,
    pub fragment_definitions: Vec<FragmentDefinition>
}

#[derive(Debug)]
pub enum Field {
    Field { alias: Option<String>, name: String, parameters: Vec<Parameter>, fields: Vec<Field> },
    Fragment { name: String }
}

#[derive(Debug)]
pub struct FragmentDefinition {
    pub name: String,
    pub r#type: String,
    pub fields: Vec<Field>
}

#[derive(Debug)]
pub struct Operation {
    pub operation_type: OperationType,
    pub name: Option<String>,
    pub variables: Vec<Variable>,
    pub fields: Vec<Field>
}

impl Field {
    pub fn new_field(alias: Option<String>, name: String, parameters: Vec<Parameter>, fields: Vec<Field>) -> Field {
        Field::Field {
            alias: alias,
            name: name,
            parameters: parameters,
            fields: fields
        }
    }

    pub fn new_fragment(name: String) -> Field {
        Field::Fragment {
            name: name
        }
    }
}

#[derive(Debug)]
pub struct Parameter {
    name: String,
    value: ParameterValue
}

#[derive(Debug)]
pub enum ParameterValue {
    Nil,
    Scalar(String),
    Object(Vec<ParameterField>),
    List(Vec<ParameterValue>),
    Variable(String)
}

#[derive(Debug)]
pub enum OperationType {
    Query, Mutation, Subscription
}

#[derive(Debug)]
pub struct ParameterField {
    name: String,
    value: ParameterValue
}

#[derive(Debug)]
pub struct Error {
    error: String
}

impl Error {
    pub fn new(error: String) -> Error {
        Error { error: error }
    }
}