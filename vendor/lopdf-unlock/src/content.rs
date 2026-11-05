use super::Object;
use crate::Result;
use crate::writer::Writer;
use std::io::Write;

#[derive(Debug, Clone)]
pub struct Operation {
    pub operator: String,
    pub operands: Vec<Object>,
}

impl Operation {
    pub fn new(operator: &str, operands: Vec<Object>) -> Operation {
        Operation {
            operator: operator.to_string(),
            operands,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Content<Operations: AsRef<[Operation]> = Vec<Operation>> {
    pub operations: Operations,
}

impl<Operations: AsRef<[Operation]>> Content<Operations> {
    /// Encode content operations.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let mut buffer = Vec::new();
        let mut first_operation = true;
        for operation in self.operations.as_ref() {
            // Add new line after each operation except the last one.
            if first_operation {
                first_operation = false;
            } else {
                buffer.write_all(b"\n")?;
            }
            if operation.operator == "BI" {
                // An inline image: `BI <entries> ID <data> EI`, the data as read.
                let Some(Object::Stream(image)) = operation.operands.first() else {
                    return Err(crate::Error::InvalidInlineImage(String::from(
                        "an inline image without its data cannot be written",
                    )));
                };
                buffer.write_all(b"BI")?;
                for (key, value) in image.dict.iter() {
                    buffer.write_all(b" ")?;
                    Writer::write_object(&mut buffer, &Object::Name(key.clone()))?;
                    buffer.write_all(b" ")?;
                    Writer::write_object(&mut buffer, value)?;
                }
                buffer.write_all(b" ID ")?;
                buffer.write_all(&image.content)?;
                buffer.write_all(b"\nEI")?;
                continue;
            }
            for operand in &operation.operands {
                Writer::write_object(&mut buffer, operand)?;
                buffer.write_all(b" ")?;
            }
            buffer.write_all(operation.operator.as_bytes())?;
        }
        Ok(buffer)
    }
}

