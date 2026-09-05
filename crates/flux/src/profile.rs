use std::io::Read;

use flux::{
    ContentKey, ContentProfile, Error, ErrorKind, ProfileId, Result, Segment, TargetPath, Validator,
};
use swifty_artifacts::{
    Md5Digest, SrfPart, SwiftyStreamingPartScanner, SwiftyStreamingPartValidator,
};

use crate::input::swifty_profile_id;

pub struct SwiftyFluxProfile;

impl ContentProfile for SwiftyFluxProfile {
    fn id(&self) -> ProfileId {
        swifty_profile_id()
    }

    fn scan(
        &self,
        path: &TargetPath,
        len: u64,
        reader: &mut dyn Read,
        emit: &mut dyn FnMut(Segment) -> Result<()>,
    ) -> Result<()> {
        let mut scanner = SwiftyStreamingPartScanner::new(path.as_str(), len);
        let mut bytes_read = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            bytes_read = bytes_read
                .checked_add(count as u64)
                .ok_or_else(|| Error::new(ErrorKind::Validation, "profile scan length overflow"))?;
            for part in scanner.push(&buffer[..count]).map_err(swifty_error)? {
                if part.length > 0 {
                    emit(part_segment(part)?)?;
                }
            }
        }
        for part in scanner.finish().map_err(swifty_error)? {
            if part.length > 0 {
                emit(part_segment(part)?)?;
            }
        }
        if bytes_read != len {
            return Err(Error::new(
                ErrorKind::Validation,
                "profile scanner read length mismatch",
            ));
        }
        Ok(())
    }

    fn validator(&self, key: &ContentKey) -> Result<Box<dyn Validator>> {
        validate_key(key)?;
        let digest =
            Md5Digest::from_bytes(key.identity().try_into().map_err(|_| {
                Error::new(ErrorKind::Validation, "invalid Swifty MD5 digest length")
            })?);
        Ok(Box::new(SwiftyValidator {
            validator: SwiftyStreamingPartValidator::new(digest, key.length()),
        }))
    }
}

fn part_segment(part: SrfPart) -> Result<Segment> {
    Ok(Segment {
        offset: part.start,
        key: ContentKey::new(
            swifty_profile_id(),
            part.checksum.as_bytes().to_vec(),
            part.length,
        )?,
    })
}

struct SwiftyValidator {
    validator: SwiftyStreamingPartValidator,
}

impl Validator for SwiftyValidator {
    fn update(&mut self, bytes: &[u8]) -> Result<()> {
        self.validator.push(bytes).map_err(swifty_error)
    }

    fn finish(self: Box<Self>) -> Result<()> {
        self.validator.finish().map_err(swifty_error)?;
        Ok(())
    }
}

fn validate_key(key: &ContentKey) -> Result<()> {
    if key.profile() != swifty_profile_id() {
        return Err(Error::new(
            ErrorKind::Validation,
            "validation profile mismatch",
        ));
    }
    if key.identity().len() != 16 {
        return Err(Error::new(
            ErrorKind::Validation,
            "invalid Swifty MD5 digest length",
        ));
    }
    Ok(())
}

fn swifty_error(error: swifty_artifacts::SwiftyError) -> Error {
    Error::with_source(ErrorKind::Validation, error)
}
