use bytes::Bytes;
use flux::{FluxError, FluxErrorKind, FluxResult, TargetPath};
use flux_content::{ProfileFileScanner, StreamingValidator, ValidationEvidence};
use swifty_artifacts::{Md5Digest, SwiftyStreamingPartValidator};

pub struct SwiftyFluxProfile;

impl flux::ContentProfile for SwiftyFluxProfile {
    fn fingerprint(&self) -> flux::ProfileFingerprint {
        crate::swifty_profile_fingerprint()
    }

    fn begin_file_inventory(
        &self,
        _path: &TargetPath,
        _len: u64,
    ) -> FluxResult<Box<dyn ProfileFileScanner>> {
        // Fleet/Swifty requires pre-determined part layout for inventory scanning.
        // Core Flux materialization does not perform Swifty-style part scanning on arbitrary files.
        // Fleet's own refresh logic handles this using swifty_artifacts.
        Err(FluxError::new(
            FluxErrorKind::Unsupported,
            "Swifty profile does not support arbitrary file inventory scanning",
        ))
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
