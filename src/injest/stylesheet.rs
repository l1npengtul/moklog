use color_eyre::Result;
use lightningcss::printer::PrinterOptions;
use lightningcss::stylesheet::{MinifyOptions, ParserOptions, StyleSheet};
use rsass::output::Format;

pub async fn compile_sass(data: &[u8]) -> Result<String> {
    let compiled = rsass::compile_scss(data, Format::default())?;
    Ok(String::from_utf8(compiled)?)
}

pub async fn optimize_css(css: &str) -> Result<String> {
    let mut stylesheet = StyleSheet::parse(css, ParserOptions::default())?;
    stylesheet.minify(MinifyOptions::default())?;
    Ok(stylesheet.to_css(PrinterOptions::default())?.code)
}
