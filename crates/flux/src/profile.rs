use bytes::Bytes;
use flux::{FluxError, FluxErrorKind, FluxResult, TargetPath};
use flux_content::{
    ProfileFileScanner, SegmentObservation, StreamingValidator, ValidationEvidence,
};
use std::sync::Arc;
use swifty_artifacts::{
    Md5Digest, SrfPart, SwiftyStreamingPartScanner, SwiftyStreamingPartValidator,
};

use crate::input::swifty_profile_fingerprint;

pub trait HashProgressObserver: Send + Sync {
    fn bytes_hashed(&self, bytes: u64);
}

pub type HashProgressObserverRef = Arc<dyn HashProgressObserver>;

pub struct SwiftyFluxProfile {
    hash_progress: Option<HashProgressObserverRef>,
}

impl SwiftyFluxProfile {
    pub fn new(hash_progress: Option<HashProgressObserverRef>) -> Self {
        Self { hash_progress }
    }
}

impl flux::ContentProfile for SwiftyFluxProfile {
    fn fingerprint(&self) -> flux::ProfileFingerprint {
        swifty_profile_fingerprint()
    }

    fn begin_file_scan(
        &self,
        path: &TargetPath,
        len: u64,
    ) -> FluxResult<Box<dyn ProfileFileScanner>> {
        Ok(Box::new(SwiftyInventoryScanner {
            scanner: SwiftyStreamingPartScanner::new(path.as_str(), len),
            hash_progress: self.hash_progress.clone(),
        }))
    }

    fn validator(&self, spec: &flux::ValidationSpec) -> FluxResult<Box<dyn StreamingValidator>> {
        validate_spec(spec)?;
        let digest = swifty_digest_from_spec(spec)?;
        Ok(Box::new(SwiftyPartValidator {
            spec: spec.clone(),
            validator: SwiftyStreamingPartValidator::new(digest, spec.len),
        }))
    }
}

struct SwiftyInventoryScanner {
    scanner: SwiftyStreamingPartScanner,
    hash_progress: Option<HashProgressObserverRef>,
}

impl ProfileFileScanner for SwiftyInventoryScanner {
    fn push(&mut self, bytes: Bytes) -> FluxResult<Vec<SegmentObservation>> {
        let byte_count = bytes.len() as u64;
        let observations = self
            .scanner
            .push(bytes.as_ref())
            .map_err(swifty_error)?
            .into_iter()
            .filter(|part| part.length > 0)
            .map(part_observation)
            .collect::<FluxResult<Vec<_>>>()?;
        if let Some(progress) = &self.hash_progress {
            progress.bytes_hashed(byte_count);
        }
        Ok(observations)
    }

    fn finish(self: Box<Self>) -> FluxResult<Vec<SegmentObservation>> {
        self.scanner
            .finish()
            .map_err(swifty_error)?
            .into_iter()
            .filter(|part| part.length > 0)
            .map(part_observation)
            .collect()
    }
}

fn part_observation(part: SrfPart) -> FluxResult<SegmentObservation> {
    let profile = swifty_profile_fingerprint();
    let key = flux::SegmentKey::new(
        profile,
        flux::OpaqueSegmentIdentity::new(part.checksum.as_bytes().to_vec())?,
        part.length,
    )?;
    Ok(SegmentObservation {
        range: part.start..part.start + part.length,
        validation: flux::ValidationSpec {
            profile,
            key: key.clone(),
            len: part.length,
        },
        key,
    })
}

struct SwiftyPartValidator {
    spec: flux::ValidationSpec,
    validator: SwiftyStreamingPartValidator,
}

impl StreamingValidator for SwiftyPartValidator {
    fn push(&mut self, bytes: Bytes) -> FluxResult<()> {
        self.validator.push(bytes.as_ref()).map_err(swifty_error)
    }

    fn finish(self: Box<Self>) -> FluxResult<ValidationEvidence> {
        let len = self.validator.finish().map_err(swifty_error)?;
        Ok(ValidationEvidence {
            spec: self.spec.clone(),
            len,
        })
    }
}

fn validate_spec(spec: &flux::ValidationSpec) -> FluxResult<()> {
    if spec.profile != swifty_profile_fingerprint() {
        return Err(FluxError::new(
            FluxErrorKind::ValidationFailed,
            "validation spec profile fingerprint mismatch",
        ));
    }
    if spec.key.profile != spec.profile {
        return Err(FluxError::new(
            FluxErrorKind::ValidationFailed,
            "validation spec key profile mismatch",
        ));
    }
    if spec.key.identity.bytes().len() != 16 {
        return Err(FluxError::new(
            FluxErrorKind::ValidationFailed,
            "invalid Swifty MD5 digest length",
        ));
    }
    if spec.key.len != spec.len {
        return Err(FluxError::new(
            FluxErrorKind::ValidationFailed,
            "validation spec key length mismatch",
        ));
    }
    Ok(())
}

fn swifty_digest_from_spec(spec: &flux::ValidationSpec) -> FluxResult<Md5Digest> {
    let bytes: [u8; 16] = spec.key.identity.bytes().try_into().map_err(|_| {
        FluxError::new(
            FluxErrorKind::ValidationFailed,
            "invalid Swifty MD5 digest length",
        )
    })?;
    Ok(Md5Digest::from_bytes(bytes))
}

fn swifty_error(error: swifty_artifacts::SwiftyError) -> FluxError {
    FluxError::new(FluxErrorKind::ValidationFailed, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{HashProgressObserver, HashProgressObserverRef, SwiftyInventoryScanner};
    use bytes::Bytes;
    use flux_content::ProfileFileScanner;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use swifty_artifacts::SwiftyStreamingPartScanner;

    struct HashByteCounter(AtomicU64);

    impl HashProgressObserver for HashByteCounter {
        fn bytes_hashed(&self, bytes: u64) {
            self.0.fetch_add(bytes, Ordering::Relaxed);
        }
    }

    #[test]
    fn scanner_reports_hashed_bytes_after_a_successful_push() {
        let counter = Arc::new(HashByteCounter(AtomicU64::new(0)));
        let hash_progress: HashProgressObserverRef = counter.clone();
        let mut scanner = SwiftyInventoryScanner {
            scanner: SwiftyStreamingPartScanner::new("example.pbo", 6),
            hash_progress: Some(hash_progress),
        };

        scanner.push(Bytes::from_static(b"abc")).unwrap();
        assert_eq!(counter.0.load(Ordering::Relaxed), 3);
        scanner.push(Bytes::from_static(b"def")).unwrap();

        assert_eq!(counter.0.load(Ordering::Relaxed), 6);
    }
}
