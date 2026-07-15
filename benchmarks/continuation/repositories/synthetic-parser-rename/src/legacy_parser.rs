#[derive(Debug, PartialEq, Eq)]
pub struct Frame<'a> {
    pub kind: &'a str,
    pub value: &'a str,
}

pub fn parse_frame(input: &str) -> Result<Frame<'_>, &'static str> {
    let (kind, value) = input.split_once(':').ok_or("frame must contain ':'")?;
    if kind.is_empty() || value.is_empty() {
        return Err("frame fields must not be empty");
    }
    Ok(Frame { kind, value })
}
