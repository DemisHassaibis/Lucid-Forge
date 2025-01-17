use nom::{
    bytes::complete::tag,
    character::complete::char,
    combinator::map,
    sequence::tuple,
    IResult,
};

use crate::cosql::common::{ws, parse_variable, parse_identifier};
use super::{Attributes, parse_attributes1};

#[derive(Debug, Clone, PartialEq)]
pub struct EntityInsertion {
    pub variable: String,
    pub entity_type: String,
    pub attributes: Attributes,
}

pub fn parse_entity_insertion(input: &str) -> IResult<&str, EntityInsertion> {
    map(
        tuple((
            ws(parse_variable),
            ws(tag("isa")),
            ws(parse_identifier),
            parse_attributes1,
            ws(char(';')),
        )),
        |(var, _, entity_type, attr, _)| EntityInsertion {
            variable: var.to_owned(),
            entity_type: entity_type.to_owned(),
            attributes: attr,
        },
    )(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cosql::{
        insertion::Attribute,
        Value,
        Date,
    };

    #[test]
    fn test_parse_entity_insertion() {
        let test_cases = [
            (
                r#"$developer isa person (
                    name: \"The Rust Developer\",
                    age: 54,
                    date_of_birth: 01-01-1970
                );"#,
                EntityInsertion {
                    variable: "developer".to_owned(),
                    entity_type: "person".to_owned(),
                    attributes: vec![
                        Attribute {
                            name: "name".to_owned(),
                            value: Value::String("The Rust Developer".to_owned()),
                        },
                        Attribute {
                            name: "age".to_owned(),
                            value: Value::Int(54),
                        },
                        Attribute {
                            name: "date_of_birth".to_owned(),
                            value: Value::Date(Date(1, 1, 1970)),
                        },
                    ],
                },
            ),
            (
                r#"$project isa initiative (
                    name: \"Rust Project\",
                    start_date: 01-01-2000,
                    end_date: 31-12-2009
                );"#,
                EntityInsertion {
                    variable: "project".to_owned(),
                    entity_type: "initiative".to_owned(),
                    attributes: vec![
                        Attribute {
                            name: "name".to_owned(),
                            value: Value::String("Rust Project".to_owned()),
                        },
                        Attribute {
                            name: "start_date".to_owned(),
                            value: Value::Date(Date(1, 1, 2000)),
                        },
                        Attribute {
                            name: "end_date".to_owned(),
                            value: Value::Date(Date(31, 12, 2009)),
                        },
                    ],
                },
            ),
        ];

        for (input, expected) in test_cases {
            let (_, result) = parse_entity_insertion(input).unwrap();
            assert_eq!(result, expected);
        }
    }
}
