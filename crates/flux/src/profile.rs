use std::io::Read;

use flux::{ContentVerifier, Error, Key, ProfileId, Recipe, Result, Validator};
use swifty_artifacts::{
    Md5Digest, SrfPart, SwiftyStreamingPartScanner, SwiftyStreamingPartValidator,
};

use relative_path::RelativePath;
pub(super) const PROFILE_ID: ProfileId = *b"fleet-swifty-md5-profile-v1.0000";
const SCAN_BUFFER: usize = 64 * 1024;

pub(super) struct SwiftyVerifier;

pub(super) fn read_recipe(
    path: &RelativePath,
    length: u64,
    reader: &mut dyn Read,
) -> Result<Recipe> {
    let mut scanner = SwiftyStreamingPartScanner::new(path.as_str(), length);
    let mut buffer = [0u8; SCAN_BUFFER];
    let mut keys = Vec::new();
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let parts = scanner.push(&buffer[..count]).map_err(swifty_error)?;
        for part in parts.into_iter().filter(|part| part.length > 0) {
            keys.push(part_key(part)?);
        }
    }
    for part in scanner
        .finish()
        .map_err(swifty_error)?
        .into_iter()
        .filter(|part| part.length > 0)
    {
        keys.push(part_key(part)?);
    }
    Recipe::new(keys)
}
impl ContentVerifier for SwiftyVerifier {
    fn validator(&self, key: &Key) -> Result<Box<dyn Validator>> {
        let digest = Md5Digest::from_bytes(
            key.identity()
                .try_into()
                .map_err(|_| Error::Validation("invalid Swifty MD5 digest length".into()))?,
        );
        Ok(Box::new(SwiftyValidator {
            validator: SwiftyStreamingPartValidator::new(digest, key.length()),
        }))
    }
}

fn part_key(part: SrfPart) -> Result<Key> {
    Key::new(part.checksum.as_bytes().to_vec(), part.length)
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

fn swifty_error(error: swifty_artifacts::SwiftyError) -> Error {
    Error::Validation(error.to_string())
}
