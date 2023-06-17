use crate::progress::{create_chunk_pb, ProgressBarType};
use sha2::{Digest, Sha256};
use sha3::Sha3_224;
use std::path::Path;
use tokio::fs::File;
use tokio::io::AsyncReadExt;

const VERSION_AND_HEADER: u64 = 0x333333334;

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct FileChunk {
    pub chunk_no: u64,
    pub pos: u64,
    pub len: u64,
    //use sha-256 (sha256) for chunk collision resistance and performance
    pub sha256: [u8; 32],
}

#[derive(Debug, PartialEq, Eq)]
pub struct FileChunkDesc {
    pub version: u64,
    pub size: u64,
    pub chunk_size: u64,
    //golem uses sha-3 (sha224) for checking integrity of downloaded images
    pub sha3: [u8; 28],
    pub chunks: Vec<FileChunk>,
}

impl FileChunkDesc {
    pub fn serialize_to_bytes(self: &FileChunkDesc) -> Vec<u8> {
        let expected_length = 8 + 8 + 8 + 28 + self.chunks.len() * 32;
        let mut bytes = Vec::with_capacity(expected_length);
        bytes.extend_from_slice(&self.version.to_be_bytes());
        bytes.extend_from_slice(&self.size.to_be_bytes());
        bytes.extend_from_slice(&self.chunk_size.to_be_bytes());
        bytes.extend_from_slice(&self.sha3);
        let number_of_chunks = (self.size + self.chunk_size - 1) / self.chunk_size;
        if number_of_chunks != self.chunks.len() as u64 {
            //sanity check
            eprintln!("File descriptor {:?}", self);
            panic!("number of chunks is not equal to size / chunk_size. This should not happen. {} vs {}", number_of_chunks, self.chunks.len());
        }
        for chunk in &self.chunks {
            bytes.extend_from_slice(&chunk.sha256);
        }
        if bytes.len() != expected_length {
            panic!(
                "Invalid descriptor created expected length {} vs {}",
                expected_length,
                bytes.len()
            );
        }
        bytes
    }

    pub fn deserialize_from_bytes(bytes: &[u8]) -> Result<Self, anyhow::Error> {
        let mut offset = 0;
        if bytes.len() < 8 {
            return Err(anyhow::anyhow!("Invalid descriptor length {}", bytes.len()));
        }
        let version_bytes = u64::from_be_bytes(bytes[offset..offset + 8].try_into()?);
        offset += 8;
        if version_bytes != VERSION_AND_HEADER {
            return Err(anyhow::anyhow!(
                "Invalid descriptor version {}",
                version_bytes
            ));
        }

        let size = u64::from_be_bytes(bytes[offset..offset + 8].try_into()?);
        offset += 8;
        let chunk_size = u64::from_be_bytes(bytes[offset..offset + 8].try_into()?);
        offset += 8;
        let number_of_chunks = ((size + chunk_size - 1) / chunk_size) as usize;
        let mut sha3 = [0_u8; 28];
        sha3.copy_from_slice(&bytes[offset..offset + 28]);
        offset += 28;

        if bytes.len() != offset + number_of_chunks * 32 {
            return Err(anyhow::anyhow!(
                "Invalid descriptor expected length {} vs {}",
                offset + number_of_chunks * 32,
                bytes.len()
            ));
        }

        let mut descr = FileChunkDesc {
            version: version_bytes,
            size,
            chunk_size,
            sha3,
            chunks: Vec::with_capacity(number_of_chunks),
        };

        let mut file_pos = 0;
        for chunk_no in 0..number_of_chunks {
            let chunk_length = std::cmp::min(chunk_size, size - file_pos);
            let mut sha256 = [0_u8; 32];
            sha256.copy_from_slice(&bytes[offset..offset + 32]);
            descr.chunks.push(FileChunk {
                chunk_no: chunk_no as u64,
                pos: file_pos,
                len: chunk_length,
                sha256,
            });
            file_pos += chunk_size;
            offset += 32;
        }
        Ok(descr)
    }
}

pub async fn create_descriptor_from_reader<AsyncReader>(
    mut reader: AsyncReader,
    file_size: u64,
    chunk_size: usize,
) -> anyhow::Result<FileChunkDesc>
where
    AsyncReader: tokio::io::AsyncRead + Unpin,
{
    let pb = create_chunk_pb(file_size, ProgressBarType::CreateDescriptor);
    pb.set_message("Reading file");

    let mut file_chunks = Vec::new();
    let mut offset = 0;
    let mut sha256_chunk = Sha256::new();
    let mut buffer = vec![0; chunk_size];
    let mut chunk_no = 0;
    let mut sha3 = Sha3_224::new();
    while offset < file_size {
        let chunk_size = std::cmp::min(chunk_size, (file_size - offset) as usize);
        buffer.resize(chunk_size, 0);

        reader.read_exact(&mut buffer).await.unwrap();
        sha256_chunk.update(&buffer);
        sha3.update(&buffer);
        let chunk = FileChunk {
            chunk_no,
            pos: offset,
            len: chunk_size as u64,
            sha256: sha256_chunk.finalize_reset().into(),
        };
        chunk_no += 1;
        file_chunks.push(chunk);
        offset += chunk_size as u64;
        pb.inc(chunk_size as u64);
    }

    Ok(FileChunkDesc {
        version: VERSION_AND_HEADER,
        size: file_size,
        chunk_size: chunk_size as u64,
        chunks: file_chunks,
        sha3: sha3.finalize().into(),
    })
}

pub async fn create_descriptor(path: &Path, chunk_size: usize) -> anyhow::Result<FileChunkDesc> {
    let file = File::open(path).await?;
    let file_size = file.metadata().await.unwrap().len();
    create_descriptor_from_reader(file, file_size, chunk_size).await
}

#[tokio::test]
async fn test_descriptor_creation() {
    let rng = fastrand::Rng::new();
    rng.seed(1234);

    use std::iter::repeat_with;

    let bytes1: Vec<u8> = repeat_with(|| rng.u8(..)).take(10000).collect();
    let bytes2: Vec<u8> = repeat_with(|| rng.u8(..)).take(10001).collect();

    let descr1 = create_descriptor_from_reader(&bytes1[..], bytes1.len() as u64, 1000)
        .await
        .unwrap();
    assert_eq!(descr1.version, VERSION_AND_HEADER);
    assert_eq!(descr1.size, 10000);
    assert_eq!(descr1.chunk_size, 1000);
    assert_eq!(descr1.chunks.len(), 10);
    let descr2 = create_descriptor_from_reader(&bytes2[..], bytes2.len() as u64, 1000)
        .await
        .unwrap();
    assert_eq!(descr2.size, 10001);
    assert_eq!(descr2.chunk_size, 1000);
    assert_eq!(descr2.chunks.len(), 11);
    let descr_empty = create_descriptor_from_reader(&bytes1[..], 0, 1000)
        .await
        .unwrap();
    assert_eq!(descr_empty.size, 0);
    assert_eq!(descr_empty.chunk_size, 1000);
    assert_eq!(descr_empty.chunks.len(), 0);
    let descr_single = create_descriptor_from_reader(&bytes1[..], 115, 1000)
        .await
        .unwrap();
    assert_eq!(descr_single.size, 115);
    assert_eq!(descr_single.chunk_size, 1000);
    assert_eq!(descr_single.chunks.len(), 1);

    for chunk in &descr1.chunks {
        println!(
            "Chunk: pos: {} len: {} hash: {}",
            chunk.pos,
            chunk.len,
            hex::encode(chunk.sha256)
        );
    }
    for chunk in &descr2.chunks {
        println!(
            "Chunk: pos: {} len: {} hash: {}",
            chunk.pos,
            chunk.len,
            hex::encode(chunk.sha256)
        );
    }

    let bytes1 = descr1.serialize_to_bytes();
    assert_eq!(bytes1.len(), 8 + 8 + 8 + 28 + 10 * 32);
    let bytes2 = descr2.serialize_to_bytes();
    assert_eq!(bytes2.len(), 8 + 8 + 8 + 28 + 11 * 32);
    let bytes_empty = descr_empty.serialize_to_bytes();
    assert_eq!(bytes_empty.len(), 8 + 8 + 8 + 28);
    let bytes_single = descr_single.serialize_to_bytes();
    assert_eq!(bytes_single.len(), 8 + 8 + 8 + 28 + 1 * 32);

    let descr_de1 = FileChunkDesc::deserialize_from_bytes(&bytes1).unwrap();
    assert_eq!(descr_de1, descr1);
    let descr_de2 = FileChunkDesc::deserialize_from_bytes(&bytes2).unwrap();
    assert_eq!(descr_de2, descr2);
    let descr_de_empty = FileChunkDesc::deserialize_from_bytes(&bytes_empty).unwrap();
    assert_eq!(descr_de_empty, descr_empty);
    let descr_de_single = FileChunkDesc::deserialize_from_bytes(&bytes_single).unwrap();
    assert_eq!(descr_de_single, descr_single);
}
