use std::io::{Read, Seek, SeekFrom};
use std::{fs, io::Write, path::Path};

use anyhow::anyhow;
use bollard::service::ContainerConfig;
use crc::{Crc, CRC_32_ISO_HDLC};

use crate::rwbuf::RWBuffer;

pub async fn add_metadata_outside(
    image_path: &Path,
    config: &ContainerConfig,
) -> anyhow::Result<usize> {
    let mut json_buf = RWBuffer::new();
    serde_json::to_writer(&mut json_buf, config)?;
    let mut file = fs::OpenOptions::new().append(true).open(image_path)?;
    let meta_size = json_buf.bytes.len();
    let crc = {
        //CRC_32_ISO_HDLC is another name of CRC-32-IEEE which was used in previous version of crc
        let crc_algo = Crc::<u32>::new(&CRC_32_ISO_HDLC);
        let mut digest = crc_algo.digest();
        digest.update(&json_buf.bytes);
        digest.finalize()
    };
    let mut bytes_written = file.write(&crc.to_le_bytes())?;
    bytes_written += file.write(&json_buf.bytes)?;
    bytes_written += file.write(format!("{meta_size:08}").as_bytes())?;
    Ok(bytes_written)
}

pub async fn read_metadata_outside(image_path: &Path) -> anyhow::Result<ContainerConfig> {
    const META_SIZE_BYTES: usize = 8;
    const CRC_BYTES: usize = 4;

    //obtain position of meta data by checking last bytes of file
    let mut file = fs::OpenOptions::new().read(true).open(image_path)?;
    file.seek(SeekFrom::End(0))?;
    let file_size = file.stream_position()?;
    if file_size < (META_SIZE_BYTES + CRC_BYTES) as u64 {
        return Err(anyhow!("File is too small"));
    }

    //read metadata from end of the file
    file.seek(SeekFrom::End(-(META_SIZE_BYTES as i64)))?;
    let mut buf = [0; META_SIZE_BYTES];
    file.read_exact(&mut buf)?;
    //parse from dec string
    let meta_size = (std::str::from_utf8(&buf)?).parse::<u64>()?;
    if meta_size + (META_SIZE_BYTES + CRC_BYTES) as u64 > file_size {
        return Err(anyhow!("File is too small"));
    }
    file.seek(SeekFrom::End(
        -((META_SIZE_BYTES + CRC_BYTES + meta_size as usize) as i64),
    ))?;
    let mut crc_buf = [0; CRC_BYTES];
    file.read_exact(&mut crc_buf)?;
    let mut json_buf = vec![0; meta_size as usize];
    file.read_exact(&mut json_buf)?;
    let crc = {
        //CRC_32_ISO_HDLC is another name of CRC-32-IEEE which was used in previous version of crc
        let crc_algo = Crc::<u32>::new(&CRC_32_ISO_HDLC);
        let mut digest = crc_algo.digest();
        digest.update(&json_buf);
        digest.finalize()
    };
    let crc_read = u32::from_le_bytes(crc_buf);
    if crc != crc_read {
        return Err(anyhow!("CRC mismatch"));
    }
    let cfg = serde_json::from_slice::<ContainerConfig>(&json_buf)?;
    Ok(cfg)
}

#[tokio::test]
async fn test_descriptor_creation() {
    let rng = fastrand::Rng::new();

    let test_file_name = &PathBuf::from("test_descriptor_creation.tst");
    rng.seed(1234);

    use std::iter::repeat_with;

    let bytes: Vec<u8> = repeat_with(|| rng.u8(..)).take(16521).collect();
    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(test_file_name)
        .unwrap();
    file.write_all(&bytes).unwrap();

    println!("Read metadata outside");
    assert_eq!(read_metadata_outside(test_file_name).await.is_err(), true);
    let mut cfg_write = ContainerConfig::default();
    cfg_write.image = Some("test".to_string());
    cfg_write.cmd = Some(vec!["test".to_string()]);
    cfg_write.entrypoint = Some(vec!["test".to_string()]);
    cfg_write.env = Some(vec!["test".to_string()]);
    cfg_write.working_dir = Some("test".to_string());
    cfg_write.user = Some("test".to_string());
    cfg_write.volumes = Some(HashMap::from([
        ("foo".to_string(), HashMap::new()),
        ("foo2".to_string(), HashMap::new()),
    ]));

    add_metadata_outside(test_file_name, &cfg_write)
        .await
        .unwrap();
    let cfg_read = read_metadata_outside(test_file_name).await.unwrap();
    println!("cfg: {:?}", cfg_read);
    assert_eq!(cfg_read, cfg_write);

    fs::remove_file(test_file_name).unwrap();
}
