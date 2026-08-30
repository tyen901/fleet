use bytes::Bytes;
use flux::{FluxError, FluxErrorKind, FluxResult, TargetPath};
use flux_content::{
    FinishedFileScan, ProfileFileScanner, SegmentObservation, StreamingValidator,
    ValidationEvidence,
};
use swifty_artifacts::{
    Md5Digest, SrfPart, SwiftyStreamingPartScanner, SwiftyStreamingPartValidator,
};

pub struct SwiftyFluxProfile;

impl flux::ContentProfile for SwiftyFluxProfile {
    fn fingerprint(&self) -> flux::ProfileFingerprint {
        crate::swifty_profile_fingerprint()
    }

    fn begin_file_inventory(
        &self,
        path: &TargetPath,
        len: u64,
    ) -> FluxResult<Box<dyn ProfileFileScanner>> {
        Ok(Box::new(SwiftyInventoryScanner {
            scanner: SwiftyStreamingPartScanner::new(path.as_str(), len),
            len,
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
    len: u64,
}

impl ProfileFileScanner for SwiftyInventoryScanner {
    fn push(&mut self, bytes: Bytes) -> FluxResult<Vec<SegmentObservation>> {
        self.scanner
            .push(bytes.as_ref())
            .map_err(swifty_error)?
            .into_iter()
            .filter(|part| part.length > 0)
            .map(part_observation)
            .collect()
    }

    fn finish(self: Box<Self>) -> FluxResult<FinishedFileScan> {
        let trailing_segments = self
            .scanner
            .finish()
            .map_err(swifty_error)?
            .into_iter()
            .filter(|part| part.length > 0)
            .map(part_observation)
            .collect::<FluxResult<Vec<_>>>()?;
        Ok(FinishedFileScan {
            trailing_segments,
            len: self.len,
        })
    }
}

fn part_observation(part: SrfPart) -> FluxResult<SegmentObservation> {
    let profile = crate::swifty_profile_fingerprint();
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
    if spec.profile != crate::swifty_profile_fingerprint() {
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
