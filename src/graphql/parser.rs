mod tokenizer;
use tokenizer::Tokenizer;
use std::collections::HashSet;

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
            curly_bracket @ Some("{") if query_shorthand => {
                query_shorthand = true;
                operations.push(parse_operation(
                    curly_bracket,
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

fn append_element<B, F>(string: &mut String, iter: &[B], f: F)
where
    F: Fn(&B, &mut String),
{
    let mut first = true;
    for field in iter {
        if first {
            first = false;
        } else {
            string.push(' ');
        }
        f(field, string);
    }
}

pub fn serialize_operation<'a>(operation: &Operation<'a>) -> String {
    internal_serialize_operation(operation, false)
}

fn internal_serialize_operation<'a>(operation: &Operation<'a>, disable_shorthand: bool) -> String {
    let mut serialized_operation = String::with_capacity(500)
        + match operation.operation_type {
            OperationType::Query if operation.variables.len() == 0 && !disable_shorthand => "",
            OperationType::Query => "query",
            OperationType::Mutation => "mutation",
            OperationType::Subscription => "subscription",
        };

    if operation.variables.len() > 0 {
        serialized_operation.push('(');
        append_element(
            &mut serialized_operation,
            &operation.variables,
            serialize_variable,
        );
        serialized_operation.push(')');
    }

    serialized_operation.push('{');
    append_element(
        &mut serialized_operation,
        &operation.fields,
        serialize_field,
    );
    serialized_operation.push('}');

    serialized_operation
}

pub fn serialize_document<'a>(document: &Document<'a>) -> String {
    let op = document.operations.iter().nth(0).unwrap();
    let mut serialized_document =
        internal_serialize_operation(op, document.fragment_definitions.len() > 0);

    append_element(
        &mut serialized_document,
        &document.fragment_definitions,
        serialize_fragment,
    );

    serialized_document
}

fn serialize_fragment<'a>(fragment: &FragmentDefinition<'a>, s1: &mut String) {
    s1.push_str("fragment ");
    s1.push_str(fragment.name);
    s1.push_str(" on ");
    s1.push_str(fragment.r#type);
    s1.push('{');

    append_element(s1, &fragment.fields, serialize_field);

    s1.push('}');
}

fn serialize_field<'a>(field: &Field<'a>, s1: &mut String) {
    match field {
        Field::Field {
            alias,
            name,
            parameters,
            fields,
        } => {
            if let Some(a) = alias {
                s1.push_str(a);
                s1.push(':');
                s1.push_str(name);
            } else {
                s1.push_str(name)
            };

            if parameters.len() > 0 {
                s1.push('(');
                append_element(s1, &parameters, serialize_parameter);
                s1.push(')');
            }

            if fields.len() > 0 {
                s1.push('{');
                append_element(s1, &fields, serialize_field);
                s1.push('}');
            }
        }
        Field::Fragment { name } => {
            s1.push_str("...");
            s1.push_str(name);
        }
    }
}

fn serialize_variable<'a>(variable: &Variable<'a>, s1: &mut String) {
    s1.push('$');
    s1.push_str(variable.name);
    s1.push(':');
    s1.push_str(variable.r#type);

    match &variable.default_value {
        Some(value) => {
            s1.push('=');
            serialize_parameter_value(value, s1);
        }
        _ => {}
    }
}

fn serialize_parameter<'a>(parameter: &Parameter<'a>, s1: &mut String) {
    s1.push_str(parameter.name);
    s1.push(':');

    serialize_parameter_value(&parameter.value, s1)
}

fn serialize_parameter_value<'a>(value: &ParameterValue<'a>, s1: &mut String) {
    match value {
        ParameterValue::Scalar(s) => s1.push_str(s),
        ParameterValue::Variable(v) => s1.push_str(&format!("${}", v)),
        ParameterValue::Nil => s1.push_str("null"),
        ParameterValue::Object(o) => {
            s1.push('{');
            append_element(s1, o, serialize_parameter_field);
            s1.push('}');
        }
        ParameterValue::List(l) => {
            s1.push('[');
            append_element(s1, l, serialize_parameter_value);
            s1.push(']');
        }
    };
}

fn serialize_parameter_field<'a>(parameter_field: &ParameterField<'a>, s1: &mut String) {
    s1.push_str(parameter_field.name);
    s1.push(':');

    serialize_parameter_value(&parameter_field.value, s1)
}

pub fn expand_operation<'a>(
    operation: Operation<'a>,
    fragment_definitions: Vec<FragmentDefinition<'a>>,
) -> Result<Operation<'a>, Error> {
    let mut new_fields = Vec::new();

    if fragment_definitions.len() == 0 {
        return Ok(operation);
    }

    for field in operation.fields {
        for f in expand_fragment(field, &fragment_definitions, &mut vec!())? {
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

fn expand_fragment<'a, 'b>(
    field: Field<'a>,
    fragments: &'b [FragmentDefinition<'a>],
    fragment_stack: &mut Vec<&'b FragmentDefinition<'a>>
) -> Result<Vec<Field<'a>>, Error> {
    let fields = match field {
        Field::Fragment { name } => {
            let mut res = Vec::new();
            let fragment = fragments
                .iter()
                .filter(|f| f.name == name)
                .nth(0)
                .unwrap();

            if fragment_stack.contains(&fragment) {
                return Err(Error::new("Recursive fragment structure".to_string()));
            }    
    
            fragment_stack.push(fragment);

            for fragment_field in fragment.fields.iter()
            {
                res.append(&mut expand_fragment(fragment_field.clone(), fragments, fragment_stack)?);
            }

            fragment_stack.pop();

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
                new_subfields.append(&mut expand_fragment(subfield, fragments, fragment_stack)?);
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
        curly_bracket @ Some("{") => (curly_bracket, None, Vec::<Variable>::new()),
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
            curly_bracket @ Some("{") => (curly_bracket, Some(name), Vec::<Variable>::new()),
            Some(s) => return Err(Error::new(format!("invalid token {}", s))),
            None => return Err(Error::new(String::from("Unexpected end of string"))),
        },
        Some("(") => {
            parser_state.hierarchy.push("(");
            let variables = parse_variables(tokens, parser_state)?;

            (tokens.next(), None, variables)
        }
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

fn is_valid_variable_type(string: &str) -> bool {
    let mut chars = string.chars();

    match chars.next_back() {
        Some(c) if c == '!' || c.is_alphabetic() => chars.all(|c| c.is_alphanumeric() || c == '_'),
        Some(_) => false,
        None => false,
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
    is_valid_variable_type(string)
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

impl<'a> Document<'a> {
    pub fn filter_operation(self, operation_name: &str) -> Result<Document<'a>, Error> {
        let op = match self
            .operations
            .into_iter()
            .filter(|o| o.name.unwrap_or("") == operation_name)
            .nth(0)
        {
            Some(o) => o,
            None => return Err(Error::new(format!("operationName ist gulen"))),
        };

        return Ok(Document {
            operations: vec![op],
            fragment_definitions: self.fragment_definitions,
        });
    }
}

struct ParserState<'a> {
    hierarchy: Vec<&'a str>,
}

#[derive(Debug, Clone)]
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

impl<'a> PartialEq for FragmentDefinition<'a> {
    fn eq(&self, other: &FragmentDefinition<'a>) -> bool {
        return self.name == other.name && self.r#type == other.r#type
    }}

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

impl<'a> Operation<'a> {    
    pub fn deduplicate_fields(&self) -> Result<Operation<'a>, Error> {
        let mut new_fields = Vec::new();
        new_fields.extend(self.fields.clone());

        let merged_fields = merge_subfields(new_fields);
        let residual_variable_names = get_variables(&merged_fields);
        let residual_variables = self.variables
            .iter()
            .filter(|v| residual_variable_names.contains(&v.name.to_string()))
            .map(|v| v.clone())
            .collect::<Vec<Variable<'a>>>();

        let op = Operation {
            name: self.name.clone(),
            operation_type: self.operation_type,
            variables: residual_variables,
            fields: merged_fields
        };

        Ok(op)
    }
}

fn get_variables(fields: &[Field]) -> HashSet<String> {
    let mut hash = HashSet::new();

    for f in fields {
        for p in f.get_parameters() {
            match &p.value {
                ParameterValue::Variable(v) => { hash.insert(v.to_string()); },
                _ => { }
            }
        }

        hash.extend(get_variables(f.get_subfields()));
    }

    hash
}

impl<'a> Traversable<'a> for Operation<'a> {
    fn traverse(&self, path: &[String]) -> Option<(Vec<&Field<'a>>, &Field<'a>)> {
        if path.len() == 0 {
            None
        } else {
            self.fields
                .iter()
                .filter(|f| path[0] == f.get_alias())
                .map(|f| f.traverse(&path[1..]))
                .filter(|o| o.is_some())
                .nth(0)
                .unwrap_or(None)
        }
    }
}

impl<'a> Traversable<'a> for Field<'a> {
    fn traverse(&self, path: &[String]) -> Option<(Vec<&Field<'a>>, &Field<'a>)> {
        if path.len() == 0 {
            Some((vec![], self))
        } else {
            match self {
                Field::Field {
                    fields: subfields, ..
                } => match subfields.iter().filter(|s| path[0] == s.get_alias()).nth(0) {
                    Some(f) => match f.traverse(&path[1..]) {
                        Some((mut traversed, field)) => {
                            traversed.insert(0, self);
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

fn merge_subfields(mut fields: Vec<Field>) -> Vec<Field> {
    let mut new_subfields = Vec::new();

    while fields.len() > 0 {
        let mut subfield = fields.pop().unwrap();

        let mut del = 0;
        for ix in 0..fields.len() {
            if fields[ix - del].is_same_field(&subfield) {
                let f = fields.swap_remove(ix - del);
                del += 1;
                subfield.merge(&f);
            }
        }

        new_subfields.push(subfield);
    }

    new_subfields
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

    pub fn is_same_field(&self, field: &Field<'a>) -> bool {
        match (self, &field) {
            (Field::Field{ name, parameters, ..}, &Field::Field{ name: name2, parameters: parameters2, ..}) => {
                name == name2 && parameters.len() == parameters2.len()
                    && parameters.iter().all(|p1| field.get_parameters().iter().any(|p2| p1 == p2))
            },
            _ => false
        }
    }

    // TODO: This can be optimized
    fn has_same_parameters(&self, field: &Field<'a>) -> bool {
        if self.get_parameters().len() != field.get_parameters().len() {
            false
        } else {
            self.get_parameters()
                .iter()
                .all(|p1| field.get_parameters().iter().any(|p2| p1 == p2))
        }
    }

    pub fn merge(&mut self, field: &Field<'a>) {
        if self.get_name() == field.get_name() && self.has_same_parameters(field) {
            match (self, field) {
                (
                    Field::Field {
                        ref mut fields,
                        ..
                    },
                    Field::Field {
                        fields: fields2,
                        ..
                    },
                ) => {
                    let mut subfields = Vec::new();
                    subfields.extend(fields.clone());
                    subfields.extend(fields2.clone());

                    *fields = merge_subfields(subfields);
                }
                _ => { }
            }
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

#[derive(Debug, Clone, PartialEq)]
pub struct Parameter<'a> {
    pub name: &'a str,
    pub value: ParameterValue<'a>,
}

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_can_parse_simple_query() {
        let query = "{field}";
        let parsed_query = parse_query(query).unwrap();

        assert_eq!(1, parsed_query.operations.len());
        assert_eq!(1, parsed_query.operations[0].fields.len());
        assert_eq!("field", parsed_query.operations[0].fields[0].get_name());
    }

    #[test]
    fn parser_can_parse_simple_query_with_spaces() {
        let query = "{ field }";
        let parsed_query = parse_query(query).unwrap();

        assert_eq!(1, parsed_query.operations.len());
        assert_eq!(1, parsed_query.operations[0].fields.len());
        assert_eq!("field", parsed_query.operations[0].fields[0].get_name());
    }

    #[test]
    fn parser_can_parse_simple_query_with_commas() {
        let query = "{ field, field2, field3 }";
        let parsed_query = parse_query(query).unwrap();

        assert_eq!(1, parsed_query.operations.len());
        assert_eq!(3, parsed_query.operations[0].fields.len());
        assert_eq!("field", parsed_query.operations[0].fields[0].get_name());
        assert_eq!("field2", parsed_query.operations[0].fields[1].get_name());
        assert_eq!("field3", parsed_query.operations[0].fields[2].get_name());
    }

    #[test]
    fn parser_can_parse_simple_query_with_subfields() {
        let query = "{ field {sub1 sub2}, field2 field3 {sub3, sub4} }";
        let parsed_query = parse_query(query).unwrap();

        assert_eq!(1, parsed_query.operations.len());
        assert_eq!(3, parsed_query.operations[0].fields.len());
        assert_eq!("field", parsed_query.operations[0].fields[0].get_name());
        assert_eq!("field2", parsed_query.operations[0].fields[1].get_name());
        assert_eq!("field3", parsed_query.operations[0].fields[2].get_name());

        assert_eq!(
            2,
            parsed_query.operations[0].fields[0].get_subfields().len()
        );
        assert_eq!(
            "sub1",
            parsed_query.operations[0].fields[0].get_subfields()[0].get_name()
        );
        assert_eq!(
            "sub2",
            parsed_query.operations[0].fields[0].get_subfields()[1].get_name()
        );

        assert_eq!(
            0,
            parsed_query.operations[0].fields[1].get_subfields().len()
        );

        assert_eq!(
            2,
            parsed_query.operations[0].fields[2].get_subfields().len()
        );
        assert_eq!(
            "sub3",
            parsed_query.operations[0].fields[2].get_subfields()[0].get_name()
        );
        assert_eq!(
            "sub4",
            parsed_query.operations[0].fields[2].get_subfields()[1].get_name()
        );
    }

    #[test]
    fn parser_can_parse_query_with_aliases() {
        let query = "{alias1: field1{subalias1: sub1 sub2}, alias2: field1}";
        let parsed_query = parse_query(query).unwrap();

        assert_eq!(1, parsed_query.operations.len());
        assert_eq!(2, parsed_query.operations[0].fields.len());
        assert_eq!("field1", parsed_query.operations[0].fields[0].get_name());
        assert_eq!("alias1", parsed_query.operations[0].fields[0].get_alias());
        assert_eq!("field1", parsed_query.operations[0].fields[1].get_name());
        assert_eq!("alias2", parsed_query.operations[0].fields[1].get_alias());

        assert_eq!(
            2,
            parsed_query.operations[0].fields[0].get_subfields().len()
        );
        assert_eq!(
            "sub1",
            parsed_query.operations[0].fields[0].get_subfields()[0].get_name()
        );
        assert_eq!(
            "subalias1",
            parsed_query.operations[0].fields[0].get_subfields()[0].get_alias()
        );
        assert_eq!(
            "sub2",
            parsed_query.operations[0].fields[0].get_subfields()[1].get_name()
        );

        assert_eq!(
            0,
            parsed_query.operations[0].fields[1].get_subfields().len()
        );
    }

    #[test]
    fn parser_can_parse_query_with_parameters() {
        let query = "{alias1: field1(p1: 10){subalias1: sub1(p2: \"asd\") sub2}, alias2: field1}";
        let parsed_query = parse_query(query).unwrap();

        assert_eq!(1, parsed_query.operations.len());
        assert_eq!(2, parsed_query.operations[0].fields.len());
        assert_eq!("field1", parsed_query.operations[0].fields[0].get_name());
        assert_eq!("alias1", parsed_query.operations[0].fields[0].get_alias());
        assert_eq!(
            1,
            parsed_query.operations[0].fields[0].get_parameters().len()
        );
        assert_eq!(
            "p1",
            parsed_query.operations[0].fields[0].get_parameters()[0].name
        );
        matches!(parsed_query.operations[0].fields[0].get_parameters()[0].value, ParameterValue::Scalar(p1) if p1 == "10");

        assert_eq!("field1", parsed_query.operations[0].fields[1].get_name());
        assert_eq!("alias2", parsed_query.operations[0].fields[1].get_alias());

        assert_eq!(
            2,
            parsed_query.operations[0].fields[0].get_subfields().len()
        );
        assert_eq!(
            "sub1",
            parsed_query.operations[0].fields[0].get_subfields()[0].get_name()
        );
        matches!(parsed_query.operations[0].fields[0].get_subfields()[0].get_parameters()[0].value, ParameterValue::Scalar(p1) if p1 == "\"asd\"");

        assert_eq!(
            "subalias1",
            parsed_query.operations[0].fields[0].get_subfields()[0].get_alias()
        );
        assert_eq!(
            "sub2",
            parsed_query.operations[0].fields[0].get_subfields()[1].get_name()
        );

        assert_eq!(
            0,
            parsed_query.operations[0].fields[1].get_subfields().len()
        );
    }

    #[test]
    fn parser_can_parse_query_with_fragments() {
        let query = "query TheQuery { users{ ...userFragment surname friends {...userFragment surname } } } fragment userFragment on User { id name }";
        let parsed_query = parse_query(query).unwrap();

        assert_eq!(1, parsed_query.operations.len());
        assert_eq!(1, parsed_query.fragment_definitions.len());

        assert_eq!("users", parsed_query.operations[0].fields[0].get_name());
        matches!(parsed_query.operations[0].fields[0].get_subfields()[0], Field::Fragment { name } if name == "userFragment");

        assert_eq!(
            "surname",
            parsed_query.operations[0].fields[0].get_subfields()[1].get_name()
        );
        assert_eq!(
            "friends",
            parsed_query.operations[0].fields[0].get_subfields()[2].get_name()
        );

        matches!(parsed_query.operations[0].fields[0].get_subfields()[2].get_subfields()[0], Field::Fragment { name } if name == "userFragment");
        assert_eq!(
            "surname",
            parsed_query.operations[0].fields[0].get_subfields()[2].get_subfields()[1].get_name()
        );
    }

    #[test]
    fn parser_preserves_spaces_in_string_parameters() {
        let query = "{field1(p:\"as              d              \")}";
        let parsed_query = parse_query(query).unwrap();

        matches!(parsed_query.operations[0].fields[0].get_parameters()[0].value, ParameterValue::Scalar(p1) if p1 == "as              d              ");
    }

    #[test]
    fn parsed_string_can_be_serialized() {
        let query = "{field1}";
        let parsed_query = parse_query(query).unwrap();
        let serialized_query = serialize_document(&parsed_query);

        assert_eq!(query, serialized_query);
    }

    #[test]
    fn parsed_string_with_two_fields_can_be_serialized() {
        let query = "{field1 field2}";
        let parsed_query = parse_query(query).unwrap();
        let serialized_query = serialize_document(&parsed_query);

        assert_eq!(query, serialized_query);
    }

    #[test]
    fn parsed_string_with_subfields_can_be_serialized() {
        let query = "{field1{subfield1 subfield2} field2}";
        let parsed_query = parse_query(query).unwrap();
        let serialized_query = serialize_document(&parsed_query);

        assert_eq!(query, serialized_query);
    }

    #[test]
    fn parsed_string_with_subfields_and_parameters_can_be_serialized() {
        let query = "{field1(p1:1){subfield1(p2:2) subfield2} field2}";
        let parsed_query = parse_query(query).unwrap();
        let serialized_query = serialize_document(&parsed_query);

        assert_eq!(query, serialized_query);
    }

    #[test]
    fn parsed_mutation_can_be_serialized() {
        let query = "mutation{addUser(id:\"123\" name:\"the name\")}";
        let parsed_query = parse_query(query).unwrap();
        let serialized_query = serialize_document(&parsed_query);

        assert_eq!(query, serialized_query);
    }

    #[test]
    fn parsed_query_with_fragment_can_be_serialized() {
        let query = "query{getUser(id:\"123\"){...frag}}fragment frag on user{id name}";
        let parsed_query = parse_query(query).unwrap();
        let serialized_query = serialize_document(&parsed_query);

        assert_eq!(query, serialized_query);
    }

    #[test]
    fn parsed_query_with_parameter_can_be_serialized() {
        let query = "{launch(id:109){id site mission{name}}}";
        let parsed_query = parse_query(query).unwrap();
        let serialized_query = serialize_document(&parsed_query);

        assert_eq!(query, serialized_query);
    }

    #[test]
    fn parsed_nameless_query_with_parameter_can_be_serialized() {
        let query = "query($launchId:Int!){launch(id:$launchId){id site mission{name}}}";
        let parsed_query = parse_query(query).unwrap();
        let serialized_query = serialize_document(&parsed_query);

        assert_eq!(query, serialized_query);
    }
}
