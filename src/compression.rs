use std::io::prelude::*;

pub fn compress_data(data: Vec<u8>) -> Vec<u8> {
    eprintln!("Compressing data...");
    let mut compressor = brotli::CompressorReader::new(data.as_slice(), 4096, 11u32, 22u32);

    let mut v = vec![];
    compressor.read_to_end(&mut v).unwrap();

    return v;
}

pub fn decompress_data(data: Vec<u8>) -> Vec<u8> {
    let mut decompressor = brotli::Decompressor::new(data.as_slice(), 4096);

    let mut v = vec![];
    decompressor.read_to_end(&mut v).unwrap();

    return v;
}
