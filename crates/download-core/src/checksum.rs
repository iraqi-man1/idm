//! File hashing for checksum verification.

use std::io::Read;
use std::path::Path;

use md5::Md5;
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha512};
use velox_types::ChecksumAlgorithm;

fn hash_with<D: Digest>(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut hasher = D::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Lower-case hex digest of a file.
pub fn hash_file(path: &Path, algo: ChecksumAlgorithm) -> std::io::Result<String> {
    match algo {
        ChecksumAlgorithm::Md5 => hash_with::<Md5>(path),
        ChecksumAlgorithm::Sha1 => hash_with::<Sha1>(path),
        ChecksumAlgorithm::Sha256 => hash_with::<Sha256>(path),
        ChecksumAlgorithm::Sha512 => hash_with::<Sha512>(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_digests() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("abc.txt");
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(
            hash_file(&p, ChecksumAlgorithm::Md5).unwrap(),
            "900150983cd24fb0d6963f7d28e17f72"
        );
        assert_eq!(
            hash_file(&p, ChecksumAlgorithm::Sha1).unwrap(),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(
            hash_file(&p, ChecksumAlgorithm::Sha256).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hash_file(&p, ChecksumAlgorithm::Sha512).unwrap(),
            concat!(
                "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a",
                "2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
            )
        );
    }
}
