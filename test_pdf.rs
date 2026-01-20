use std::fs;

fn main() -> anyhow::Result<()> {
    let bytes = fs::read("/tmp/menu.pdf")?;
    let text = pdf_extract::extract_text_from_mem(&bytes)?;
    
    println!("=== FULL PDF TEXT ===");
    for (i, line) in text.lines().enumerate() {
        if !line.trim().is_empty() {
            println!("{}: {:?}", i, line);
        }
    }
    
    Ok(())
}
