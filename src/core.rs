use std::io::prelude::*;
use std::io;


pub fn compress<S: Read + Seek, W: Write + Seek>(mut input : S, mut output : W) -> io::Result<()> {
    let _ = io::copy(&mut input, &mut output);
    Ok(())
}

pub fn decompress<S: Read + Seek, W: Write + Seek>(mut input : S, mut output : W) -> io::Result<()> {
    let _ = io::copy(&mut input, &mut output);
    Ok(())
}

