//! Native CID encoding with shared length-aware CMap parsing and decoding.
use super::character_code::{CharacterCode, CodeSpace};
use super::cmap_program::{Kind, Program, Result};
use crate::{PdfDictionary, PdfObject, PdfReader};
#[cfg(test)]
#[path = "cid_encoding_tests.rs"]
mod tests;

#[derive(Debug, Clone)]
pub(crate) struct CidEncoding {
    pub(super) program: std::sync::Arc<Program>,
    pub(super) predefined_name: Option<&'static str>,
    pub code_size: u8,
    pub wmode: Option<u8>,
}
impl CidEncoding {
    fn from_program(program: Program) -> Self {
        Self {
            code_size: program.space.fixed_length().unwrap_or(0),
            wmode: program.wmode,
            program: std::sync::Arc::new(program),
            predefined_name: None,
        }
    }
    pub fn load(font: &PdfDictionary, reader: Option<&PdfReader>) -> Result<Option<Self>> {
        let Some(encoding) = font.get("Encoding") else {
            return Ok(None);
        };
        let encoding = super::cmap_stream::resolve(encoding, reader)?;
        if let PdfObject::Name(name) = &encoding {
            let info = super::predefined_cmap::lookup(name)
                .ok_or_else(|| format!("unregistered CID Encoding CMap: {name}"))?;
            let program = super::predefined_cmap::load_program(name, Kind::Cid)?;
            return Ok(Some(Self {
                code_size: program.space.fixed_length().unwrap_or(0),
                wmode: program.wmode,
                program,
                predefined_name: Some(info.name),
            }));
        }
        Self::read(&encoding, reader, 0).map(Some)
    }
    #[cfg(test)]
    pub fn cid(&self, code: u16) -> u16 {
        CharacterCode::new(u32::from(code), self.code_size)
            .map(|code| self.program.cid(code))
            .unwrap_or(0)
    }
    pub fn cid_code(&self, code: CharacterCode) -> u16 {
        self.program.cid(code)
    }
    pub fn shared_space(&self) -> std::sync::Arc<CodeSpace> {
        std::sync::Arc::clone(&self.program.space)
    }
    fn read(object: &PdfObject, reader: Option<&PdfReader>, depth: usize) -> Result<Self> {
        super::cmap_stream::read(object, reader, Kind::Cid, depth).map(Self::from_program)
    }
    #[cfg(test)]
    pub fn parse(bytes: &[u8], inherited: Option<Self>) -> Result<Self> {
        Program::parse(
            bytes,
            Kind::Cid,
            inherited.map(|base| (*base.program).clone()),
            false,
        )
        .map(Self::from_program)
    }
}
