use std::{
    io::{copy, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use android_sparse_image::{
    split::split_image, ChunkHeader, ChunkHeaderBytes, FileHeader, FileHeaderBytes,
    CHUNK_HEADER_BYTES_LEN, FILE_HEADER_BYTES_LEN,
};
use anyhow::Context;
use clap::Parser;

#[derive(clap::Parser)]
enum Opts {
    /// Inspect the contents of a sparse image
    Inspect { img: PathBuf },
    /// Expand the content of <img> to <out>
    Expand { img: PathBuf, out: PathBuf },
    /// split content of <img> to fit maximum download size
    Split {
        img: PathBuf,
        size: u32,
        out: PathBuf,
    },
}

fn inspect(img: &Path) -> anyhow::Result<()> {
    let mut file = std::fs::File::open(img)?;
    let mut header_bytes = FileHeaderBytes::default();
    file.read_exact(&mut header_bytes)?;

    let header = FileHeader::from_bytes(&header_bytes)?;
    println!(
        "Chunks {}, Expanded size: {} ({} blocks, {} blocksize), checksum: {}:",
        header.chunks,
        header.total_size(),
        header.blocks,
        header.block_size,
        header.checksum
    );
    let mut offset: usize = 0;
    for index in 0..header.chunks {
        let mut chunk_bytes = ChunkHeaderBytes::default();
        file.read_exact(&mut chunk_bytes)?;
        let chunk = ChunkHeader::from_bytes(&chunk_bytes)?;

        let out_size = chunk.out_size(&header);
        match chunk.chunk_type {
            android_sparse_image::ChunkType::Raw => {
                println!("{index}: Offset: {offset} - Copying {out_size} bytes");
                file.seek(std::io::SeekFrom::Current(chunk.data_size().try_into()?))?;
            }
            android_sparse_image::ChunkType::Fill => {
                let mut fill = [0u8; 4];
                file.read_exact(&mut fill)?;
                println!("{index}: Offset: {offset} - Filling {out_size} bytes with {fill:x?}");
            }
            android_sparse_image::ChunkType::DontCare => {
                println!("{index}: Offset: {offset} - Skipping {out_size} bytes");
            }
            android_sparse_image::ChunkType::Crc32 => {
                let mut crc = [0u8; 4];
                file.read_exact(&mut crc)?;
                println!("{index}: CRC value: {:x?}", crc);
            }
        }

        offset += out_size;
    }
    Ok(())
}

fn expand(img: &Path, out: &Path) -> anyhow::Result<()> {
    let mut file = std::fs::File::open(img)?;
    let output = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(out)?;
    let mut header_bytes: FileHeaderBytes = [0; FILE_HEADER_BYTES_LEN];
    file.read_exact(&mut header_bytes)?;

    let mut output = std::io::BufWriter::new(output);
    let header = FileHeader::from_bytes(&header_bytes)?;
    for _ in 0..header.chunks {
        let mut chunk_bytes: ChunkHeaderBytes = [0; CHUNK_HEADER_BYTES_LEN];
        file.read_exact(&mut chunk_bytes)?;
        let chunk = ChunkHeader::from_bytes(&chunk_bytes)?;

        let out_size = chunk.out_size(&header);
        match chunk.chunk_type {
            android_sparse_image::ChunkType::Raw => {
                let mut raw = (&mut file).take(out_size.try_into().unwrap());
                copy(&mut raw, &mut output)?;
            }
            android_sparse_image::ChunkType::Fill => {
                let mut fill = [0u8; 4];
                file.read_exact(&mut fill)?;
                for _ in 0..out_size / 4 {
                    output.write_all(&fill)?;
                }
            }
            android_sparse_image::ChunkType::DontCare => {
                output.seek(SeekFrom::Current(out_size.try_into().unwrap()))?;
            }
            android_sparse_image::ChunkType::Crc32 => {
                println!("Ignoring CRC");
            }
        }
    }
    output.flush()?;
    Ok(())
}

fn split(img: &Path, size: u32, out: &Path) -> anyhow::Result<()> {
    let mut file = std::fs::File::open(img)?;
    let mut header_bytes: FileHeaderBytes = [0; FILE_HEADER_BYTES_LEN];
    file.read_exact(&mut header_bytes)?;

    // Scan all chunks
    let header = FileHeader::from_bytes(&header_bytes)?;
    let mut chunks = vec![];
    for _ in 0..header.chunks {
        let mut chunk_bytes: ChunkHeaderBytes = [0; CHUNK_HEADER_BYTES_LEN];
        file.read_exact(&mut chunk_bytes)?;
        let chunk = ChunkHeader::from_bytes(&chunk_bytes)?;

        file.seek(SeekFrom::Current(chunk.data_size() as i64))?;
        chunks.push(chunk);
    }

    let splits = split_image(&header, &chunks, size)?;
    for (i, split) in splits.iter().enumerate() {
        let mut out = out.as_os_str().to_os_string();
        out.push(format!(".{i}"));
        let mut out =
            std::fs::File::create(&out).with_context(|| format!("Failed to create {out:?}"))?;
        out.write_all(&split.header.to_bytes())?;
        for chunk in &split.chunks {
            out.write_all(&chunk.header.to_bytes())?;

            file.seek(SeekFrom::Start(chunk.offset as u64))
                .context("Failed to seek input file")?;
            std::io::copy(&mut (&mut file).take(chunk.size as u64), &mut out)?;
        }
    }

    Ok(())
}

fn main() -> anyhow::Result<()> {
    let opts = Opts::parse();
    match opts {
        Opts::Inspect { img } => inspect(&img)?,
        Opts::Expand { img, out } => expand(&img, &out)?,
        Opts::Split { img, size, out } => split(&img, size, &out)?,
    }

    Ok(())
}
