use anyhow::Error;
use wasm_encoder::reencode::Reencode;

pub struct RelocatingReencoder {}

impl Reencode for RelocatingReencoder {
    type Error = Error;
}
