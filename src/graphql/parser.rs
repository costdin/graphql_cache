mod tokenizer;
use tokenizer::Tokenizer;

pub fn parse_query<'a>(query: &'a str) -> Result<Document<'a>, Error> {
    let mut operations = Vec::<Operation>::new();
    let mut fragment_definitions = Vec::<FragmentDefinition>::new();
    let mut parser_state = ParserState {
        hierarchy: Vec::<&str>::new(),
    };
    let mut tokens = Tokenizer::new(&query);

    let mut query_shorthand = true;

    loop {
        match tokens.next() {
            Some("query") => {
                query_shorthand = false;
                operations.push(parse_operation(
                    tokens.next(),
                    query_shorthand,
                    OperationType::Query,
                    &mut tokens,
                    &mut parser_state,
                )?)
            }
            Some("mutation") => {
                query_shorthand = false;
                operations.push(parse_operation(
                    tokens.next(),
                    query_shorthand,
                    OperationType::Mutation,
                    &mut tokens,
                    &mut parser_state,
                )?)
            }
            Some("subscription") => {
                query_shorthand = false;
                operations.push(parse_operation(
                    tokens.next(),
                    query_shorthand,
                    OperationType::Subscription,
                    &mut tokens,
                    &mut parser_state,
                )?)
            }
            Some("fragment") => fragment_definitions
                .push(parse_fragment_definition(&mut tokens, &mut parser_state)?),
            Some("{") if query_shorthand => {
                query_shorthand = true;
                operations.push(parse_operation(
                    Some("{"),
                    query_shorthand,
                    OperationType::Query,
                    &mut tokens,
                    &mut parser_state,
                )?)
            }
            Some("{") => {
                return Err(Error {
                    error: String::from("Operation type is required when not in shorthand mode"),
                })
            }
            Some(s) => return Err(Error::new(format!("invalid token \"{}\"", s))),
            None if operations.len() > 0 => {
                return Ok(Document {
                    operations: operations,
                    fragment_definitions: fragment_definitions,
                })
            }
            None => return Err(Error::new(String::from("Unexpected end of string"))),
        };

        if query_shorthand && operations.len() > 1 {
            return Err(Error {
                error: String::from("Only one operation allowed in shorthand mode"),
            });
        }
    }
}

pub fn expand_operation<'a>(
    operation: Operation<'a>,
    fragment_definitions: &Vec<FragmentDefinition<'a>>,
) -> Result<Operation<'a>, Error> {
    let mut new_fields = Vec::new();

    if fragment_definitions.len() == 0 {
        return Ok(operation);
    }

    for field in operation.fields {
        for f in expand_fragment(field, fragment_definitions)? {
            new_fields.push(f);
        }
    }

    return Ok(Operation {
        operation_type: operation.operation_type,
        name: operation.name,
        fields: new_fields,
        variables: operation.variables,
    });
}

fn expand_fragment<'a>(
    field: Field<'a>,
    fragments: &Vec<FragmentDefinition<'a>>,
) -> Result<Vec<Field<'a>>, Error> {
    let fields = match field {
        Field::Fragment { name } => {
            let mut res = Vec::new();
            for fragment_field in fragments
                .iter()
                .filter(|f| f.name == name)
                .nth(0)
                .unwrap()
                .fields
                .iter()
            {
                res.append(&mut expand_fragment(fragment_field.clone(), fragments)?);
            }

            res
        }
        Field::Field {
            alias,
            name,
            parameters,
            fields: subfields,
        } => {
            let mut new_subfields = vec![];
            for subfield in subfields {
                new_subfields.append(&mut expand_fragment(subfield, fragments)?);
            }

            vec![Field::Field {
                alias: alias,
                name: name,
                parameters: parameters,
                fields: new_subfields,
            }]
        }
    };

    Ok(fields)
}

fn parse_fragment_definition<'a, I>(
    tokens: &mut I,
    parser_state: &mut ParserState<'a>,
) -> Result<FragmentDefinition<'a>, Error>
where
    I: Iterator<Item = &'a str>,
{
    let name = match tokens.next() {
        Some(name) if is_valid_name(&name) => name,
        Some(s) => return Err(Error::new(format!("invalid token {}", s))),
        None => return Err(Error::new(String::from("Unexpected end of string"))),
    };

    match tokens.next() {
        Some("on") => {}
        Some(s) => return Err(Error::new(format!("invalid token {}", s))),
        None => return Err(Error::new(String::from("Unexpected end of string"))),
    };

    let type_name = match tokens.next() {
        Some(type_name) if is_valid_name(&type_name) => type_name,
        Some(s) => return Err(Error::new(format!("invalid token {}", s))),
        None => return Err(Error::new(String::from("Unexpected end of string"))),
    };

    let fields = match tokens.next() {
        Some("{") => {
            parser_state.hierarchy.push("{");
            parse_fields(tokens, parser_state)?
        }
        Some(s) => return Err(Error::new(format!("invalid token {}", s))),
        None => return Err(Error::new(String::from("Unexpected end of string"))),
    };

    Ok(FragmentDefinition {
        name: name,
        r#type: type_name,
        fields: fields,
    })
}

fn parse_operation<'a, I>(
    current_token: Option<&'a str>,
    query_shorthand: bool,
    operation_type: OperationType,
    tokens: &mut I,
    parser_state: &mut ParserState<'a>,
) -> Result<Operation<'a>, Error>
where
    I: Iterator<Item = &'a str>,
{
    let (next_token, operation_name, variables) = match current_token {
        Some("{") => (Some("{"), None, Vec::<Variable>::new()),
        Some(_) if query_shorthand => {
            return Err(Error {
                error: String::from("Operation name is not allowed in shorthand mode"),
            })
        }
        Some(name) if is_valid_name(&name) => match tokens.next() {
            Some("(") => {
                parser_state.hierarchy.push("(");
                let variables = parse_variables(tokens, parser_state)?;

                (tokens.next(), Some(name), variables)
            }
            Some("{") => (Some("{"), Some(name), Vec::<Variable>::new()),
            Some(s) => return Err(Error::new(format!("invalid token {}", s))),
            None => return Err(Error::new(String::from("Unexpected end of string"))),
        },
        Some(s) => return Err(Error::new(format!("invalid token {}", s))),
        None => return Err(Error::new(String::from("Unexpected end of string"))),
    };

    match next_token {
        Some("{") => {
            parser_state.hierarchy.push("{");
            let fields = parse_fields(tokens, parser_state)?;

            return Ok(Operation {
                operation_type: operation_type,
                name: operation_name,
                fields: fields,
                variables: variables,
            });
        }
        Some("}") => {
            return Err(Error {
                error: String::from("Unmatched parenteses"),
            })
        }
        None => return Err(Error::new(String::from("Unexpected end of string"))),
        Some(t) => return Err(Error::new(format!("invalid token {}", t))),
    }
}

fn parse_variables<'a, I>(
    tokens: &mut I,
    parser_state: &mut ParserState<'a>,
) -> Result<Vec<Variable<'a>>, Error>
where
    I: Iterator<Item = &'a str>,
{
    let mut variables = Vec::<Variable>::new();

    let mut next_token = tokens.next();
    loop {
        match next_token {
            Some(")") if is_matching_close_parenteses(")", parser_state.hierarchy.pop()) => {
                return Ok(variables)
            }
            Some(")") => {
                return Err(Error {
                    error: String::from("Unmatched parenteses"),
                })
            }
            Some("$") => {
                let name = match tokens.next() {
                    Some(n) if is_valid_name(&n) => n,
                    Some(n) => return Err(Error::new(format!("invalid variable name {}", n))),
                    None => return Err(Error::new(String::from("Unexpected end of string"))),
                };

                match tokens.next() {
                    Some(":") => {}
                    Some(n) => return Err(Error::new(format!("invalid token {}", n))),
                    None => return Err(Error::new(String::from("Unexpected end of string"))),
                };

                let variable_type = match tokens.next() {
                    Some(n) if is_valid_type(&n, tokens, parser_state) => n,
                    Some(n) => return Err(Error::new(format!("invalid type {}", n))),
                    None => return Err(Error::new(String::from("Unexpected end of string"))),
                };

                let default_value = match tokens.next() {
                    Some("=") => match tokens.next() {
                        Some(v) if is_valid_value(&v) => {
                            next_token = tokens.next();
                            Some(ParameterValue::Scalar(v))
                        }
                        Some(n) => return Err(Error::new(format!("invalid variable value {}", n))),
                        None => return Err(Error::new(String::from("Unexpected end of string"))),
                    },
                    Some(n) => {
                        next_token = Some(n);
                        None
                    }
                    None => return Err(Error::new(String::from("Unexpected end of string"))),
                };

                variables.push(Variable {
                    name: name,
                    r#type: variable_type,
                    default_value: default_value,
                });
            }
            Some(s) => return Err(Error::new(format!("invalid token {}", s))),
            None => return Err(Error::new(String::from("Unexpected end of string"))),
        };
    }
}

fn parse_fields<'a, I>(
    tokens: &mut I,
    parser_state: &mut ParserState<'a>,
) -> Result<Vec<Field<'a>>, Error>
where
    I: Iterator<Item = &'a str>,
{
    let mut fields = Vec::<Field>::new();
    let mut next_token = tokens.next();

    loop {
        let new_field = match next_token {
            Some("...") => match tokens.next() {
                Some(fragment_name) if is_valid_name(&fragment_name) => {
                    next_token = tokens.next();

                    Field::new_fragment(fragment_name)
                }
                Some(t) => return Err(Error::new(format!("invalid token {}", t))),
                None => return Err(Error::new(String::from("Unexpected end of string"))),
            },
            Some(candidate_name) if is_valid_name(&candidate_name) => {
                next_token = tokens.next();

                let (alias, name) = match next_token {
                    Some(":") => match tokens.next() {
                        Some(n) if is_valid_name(&n) => {
                            next_token = tokens.next();
                            (Some(candidate_name), n)
                        }
                        Some(s) => return Err(Error::new(format!("invalid token {}", s))),
                        None => return Err(Error::new(String::from("Unexpected end of string"))),
                    },
                    None => return Err(Error::new(String::from("Unexpected end of string"))),
                    _ => (None, candidate_name),
                };

                let parameters = match next_token {
                    Some("(") => {
                        parser_state.hierarchy.push("(");
                        let params = parse_parameters(tokens, parser_state)?;
                        next_token = tokens.next();

                        params
                    }
                    None => return Err(Error::new(String::from("Unexpected end of string"))),
                    _ => Vec::<Parameter>::new(),
                };

                let subfields = match next_token {
                    Some("{") => {
                        parser_state.hierarchy.push("{");
                        let flds = parse_fields(tokens, parser_state)?;
                        next_token = tokens.next();

                        flds
                    }
                    _ => Vec::<Field>::new(),
                };

                Field::new_field(alias, name, parameters, subfields)
            }
            Some(s) => return Err(Error::new(format!("invalid token {}", s))),
            None => return Err(Error::new(String::from("Unexpected end of string"))),
        };

        fields.push(new_field);

        match next_token {
            Some("}") if is_matching_close_parenteses("}", parser_state.hierarchy.pop()) => {
                return Ok(fields)
            }
            Some("}") => {
                return Err(Error {
                    error: String::from("Unmatched parenteses"),
                })
            }
            _ => {}
        };
    }
}

fn parse_parameters<'a, I>(
    tokens: &mut I,
    parser_state: &mut ParserState<'a>,
) -> Result<Vec<Parameter<'a>>, Error>
where
    I: Iterator<Item = &'a str>,
{
    let mut parameters = Vec::<Parameter>::new();

    loop {
        let name = match tokens.next() {
            Some(")") if parameters.len() == 0 => {
                return Err(Error::new(format!("list of parameters can't be empty")))
            }
            Some(")") if is_matching_close_parenteses(")", parser_state.hierarchy.pop()) => {
                return Ok(parameters)
            }
            Some(")") => {
                return Err(Error {
                    error: String::from("Unmatched parenteses"),
                })
            }
            Some(s) if is_valid_name(&s) => s,
            Some(s) => return Err(Error::new(format!("invalid token {}", s))),
            None => return Err(Error::new(String::from("Unexpected end of string"))),
        };

        match tokens.next() {
            Some(":") => {}
            Some(s) => return Err(Error::new(format!("invalid token {}", s))),
            None => return Err(Error::new(String::from("Unexpected end of string"))),
        };

        let value = match tokens.next() {
            Some("{") => {
                parser_state.hierarchy.push("{");
                parse_object(tokens, parser_state)?
            }
            Some("[") => {
                parser_state.hierarchy.push("[");
                parse_list(tokens, parser_state)?
            }
            Some("$") => match tokens.next() {
                Some(variable_name) if is_valid_name(&variable_name) => {
                    ParameterValue::Variable(variable_name)
                }
                Some(s) => return Err(Error::new(format!("invalid token {}", s))),
                None => return Err(Error::new(String::from("Unexpected end of string"))),
            },
            Some(s) if is_valid_value(&s) => ParameterValue::Scalar(s),
            Some(s) => return Err(Error::new(format!("invalid token {}", s))),
            None => return Err(Error::new(String::from("Unexpected end of string"))),
        };

        parameters.push(Parameter {
            name: name,
            value: value,
        });
    }
}

fn parse_object<'a, I>(
    tokens: &mut I,
    parser_state: &mut ParserState<'a>,
) -> Result<ParameterValue<'a>, Error>
where
    I: Iterator<Item = &'a str>,
{
    let mut fields = Vec::<ParameterField>::new();

    loop {
        let name = match tokens.next() {
            Some(s)
                if (s == "}" || s == ")")
                    && is_matching_close_parenteses(&s, parser_state.hierarchy.pop()) =>
            {
                return Ok(ParameterValue::Object(fields))
            }
            Some(s) if s == "}" || s == ")" => {
                return Err(Error {
                    error: String::from("Unmatched parenteses"),
                })
            }
            Some(s)
                if s == "]" && is_matching_close_parenteses(&s, parser_state.hierarchy.pop()) =>
            {
                return Ok(ParameterValue::Nil)
            }
            Some(s) => s,
            None => return Err(Error::new(String::from("Unexpected end of string"))),
        };

        match tokens.next() {
            Some(":") => {}
            Some(s) => return Err(Error::new(format!("invalid token {}", s))),
            None => return Err(Error::new(String::from("Unexpected end of string"))),
        };

        let value = match tokens.next() {
            Some("{") => {
                parser_state.hierarchy.push("{");
                parse_object(tokens, parser_state)?
            }
            Some("[") => {
                parser_state.hierarchy.push("[");
                parse_list(tokens, parser_state)?
            }
            Some(s) if is_valid_value(&s) => ParameterValue::Scalar(s),
            Some(s) => return Err(Error::new(format!("invalid token {}", s))),
            None => return Err(Error::new(String::from("Unexpected end of string"))),
        };

        fields.push(ParameterField {
            name: name,
            value: value,
        });
    }
}

fn parse_list<'a, I>(
    tokens: &mut I,
    parser_state: &mut ParserState<'a>,
) -> Result<ParameterValue<'a>, Error>
where
    I: Iterator<Item = &'a str>,
{
    let mut objs = Vec::<ParameterValue>::new();

    loop {
        match tokens.next() {
            Some("{") => {
                parser_state.hierarchy.push("{");
                objs.push(parse_object(tokens, parser_state)?);
            }
            Some("]") if is_matching_close_parenteses("]", parser_state.hierarchy.pop()) => {
                return Ok(ParameterValue::List(objs))
            }
            Some(s) if is_valid_value(&s) => objs.push(ParameterValue::Scalar(s)),
            Some(s) => return Err(Error::new(format!("invalid token {}", s))),
            None => return Err(Error::new(String::from("Unexpected end of string"))),
        }
    }
}

fn is_valid_name(string: &str) -> bool {
    let mut chars = string.chars();

    return match chars.next() {
        Some(c) if !c.is_alphabetic() => false,
        Some(_) => chars.all(|c| c.is_alphanumeric() || c == '_'),
        None => false,
    };
}

fn is_valid_value(string: &str) -> bool {
    let mut chars = string.chars();

    return match chars.next() {
        Some('"') => chars.last().unwrap_or(' ') == '"',
        Some(c) if c.is_alphanumeric() => chars.all(|c| c.is_alphanumeric()),
        _ => false,
    };
}

fn is_valid_type<'a, I>(string: &str, _tokens: &mut I, _parser_state: &mut ParserState<'a>) -> bool
where
    I: Iterator<Item = &'a str>,
{
    is_valid_name(string)
}

fn is_matching_close_parenteses(close: &str, open_option: Option<&str>) -> bool {
    match open_option {
        Some(open) => {
            (close == "}" && open == "{")
                || (close == ")" && open == "(")
                || (close == "]" && open == "[")
        }
        None => false,
    }
}

struct ParserState<'a> {
    hierarchy: Vec<&'a str>,
}

#[derive(Debug)]
pub struct Variable<'a> {
    pub name: &'a str,
    pub r#type: &'a str,
    pub default_value: Option<ParameterValue<'a>>,
}

#[derive(Debug)]
pub struct Document<'a> {
    pub operations: Vec<Operation<'a>>,
    pub fragment_definitions: Vec<FragmentDefinition<'a>>,
}

#[derive(Debug, Clone)]
pub enum Field<'a> {
    Field {
        alias: Option<&'a str>,
        name: &'a str,
        parameters: Vec<Parameter<'a>>,
        fields: Vec<Field<'a>>,
    },
    Fragment {
        name: &'a str,
    },
}

#[derive(Debug)]
pub struct FragmentDefinition<'a> {
    pub name: &'a str,
    pub r#type: &'a str,
    pub fields: Vec<Field<'a>>,
}

#[derive(Debug)]
pub struct Operation<'a> {
    pub operation_type: OperationType,
    pub name: Option<&'a str>,
    pub variables: Vec<Variable<'a>>,
    pub fields: Vec<Field<'a>>,
}

pub trait Traversable<'a> {
    fn traverse(&self, path: &[String]) -> Option<(Vec<&Field<'a>>, &Field<'a>)>;
}

impl<'a> Traversable<'a> for Operation<'a> {
    fn traverse(&self, path: &[String]) -> Option<(Vec<&Field<'a>>, &Field<'a>)> {
        if path.len() == 0 {
            None
        } else {
            self.fields
                .iter()
                .map(|f| f.traverse(path))
                .filter(|o| o.is_some())
                .nth(0)
                .unwrap_or(None)
        }
    }
}

impl<'a> Traversable<'a> for Field<'a> {
    fn traverse(&self, path: &[String]) -> Option<(Vec<&Field<'a>>, &Field<'a>)> {
        if path.len() == 0 || path[0] != self.get_alias() {
            None
        } else if path.len() == 1 {
            Some((vec![], self))
        } else {
            match self {
                Field::Field {
                    fields: subfields, ..
                } => match subfields.iter().filter(|s| path[0] == s.get_alias()).nth(0) {
                    Some(f) => match f.traverse(&path[1..]) {
                        Some((mut traversed, field)) => {
                            traversed.push(self);
                            Some((traversed, field))
                        }
                        None => None,
                    },
                    None => None,
                },
                _ => None,
            }
        }
    }
}

impl<'a> Field<'a> {
    pub fn new_field(
        alias: Option<&'a str>,
        name: &'a str,
        parameters: Vec<Parameter<'a>>,
        fields: Vec<Field<'a>>,
    ) -> Field<'a> {
        Field::Field {
            alias: alias,
            name: name,
            parameters: parameters,
            fields: fields,
        }
    }

    pub fn new_fragment(name: &'a str) -> Field<'a> {
        Field::Fragment { name: name }
    }

    pub fn has_parameters(&self) -> bool {
        match self {
            Field::Field { parameters, .. } => parameters.len() > 0,
            _ => false,
        }
    }

    pub fn has_alias(&self) -> bool {
        match self {
            Field::Field { alias, .. } => alias.is_some(),
            _ => false,
        }
    }

    pub fn children_with_parameters(&self) -> Vec<Vec<&Field>> {
        match self {
            Field::Field { fields, .. } => {
                let mut result = vec![];
                for subfield in fields {
                    if subfield.has_parameters() {
                        result.push(vec![subfield]);
                    }

                    for mut subresult in subfield.children_with_parameters() {
                        subresult.insert(0, subfield);
                        result.push(subresult);
                    }
                }

                result
            }
            _ => vec![],
        }
    }

    pub fn is_leaf(&self) -> bool {
        match self {
            Field::Field { fields, .. } => fields.len() == 0,
            _ => false,
        }
    }

    pub fn get_name(&self) -> &str {
        match self {
            Field::Field { name, .. } => name,
            _ => &"",
        }
    }

    pub fn get_alias(&self) -> &str {
        match self {
            Field::Field { alias, name, .. } => alias.unwrap_or(name),
            _ => &"",
        }
    }

    pub fn get_parameters(&self) -> &[Parameter] {
        match self {
            Field::Field { parameters, .. } => &parameters,
            _ => EMPTY_PARAMETER_LIST,
        }
    }

    pub fn get_subfields(&self) -> &[Field<'a>] {
        match self {
            Field::Field { fields, .. } => &fields,
            _ => EMPTY_FIELD_LIST,
        }
    }
}

static EMPTY_PARAMETER_LIST: &'static [Parameter] = &[];
static EMPTY_FIELD_LIST: &'static [Field] = &[];

#[derive(Debug, Clone)]
pub struct Parameter<'a> {
    name: &'a str,
    value: ParameterValue<'a>,
}

#[derive(Debug, Clone)]
pub enum ParameterValue<'a> {
    Nil,
    Scalar(&'a str),
    Object(Vec<ParameterField<'a>>),
    List(Vec<ParameterValue<'a>>),
    Variable(&'a str),
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum OperationType {
    Query,
    Mutation,
    Subscription,
}

#[derive(Debug, Clone)]
pub struct ParameterField<'a> {
    name: &'a str,
    value: ParameterValue<'a>,
}

#[derive(Debug)]
pub struct Error {
    error: String,
}

impl Error {
    pub fn new(error: String) -> Error {
        Error { error: error }
    }
}

impl From<serde_json::error::Error> for Error {
    fn from(_err: serde_json::error::Error) -> Error {
        Error {
            error: String::from("deserialization error"),
        }
    }
}