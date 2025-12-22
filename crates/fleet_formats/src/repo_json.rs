use anyhow::Result;
use serde::Deserialize;

#[derive(Clone, Debug)]
pub struct RepoSpec {
    pub repo_name: String,
    pub checksum: String,
    pub required_mods: Vec<RepoMod>,
    pub optional_mods: Vec<RepoMod>,
    pub client_parameters: String,
    pub basic_auth: Option<RepoBasicAuth>,
    pub version: String,
    pub servers: Vec<RepoServer>,
}

#[derive(Clone, Debug)]
pub struct RepoServer {
    pub name: String,
    pub address: String,
    pub port: u16,
    pub password: String,
    pub battle_eye: bool,
}

#[derive(Clone, Debug)]
pub struct RepoMod {
    pub mod_name: String,
    pub checksum: crate::digest::Md5Digest,
    pub enabled: bool,
}

#[derive(Clone, Debug)]
pub struct RepoBasicAuth {
    pub username: String,
    pub password: String,
}

#[derive(Deserialize)]
struct RawRepo {
    #[serde(rename = "repoName")]
    repo_name: String,
    #[serde(default)]
    checksum: String,
    #[serde(default, rename = "requiredMods")]
    required_mods: Vec<RawRepoMod>,
    #[serde(default, rename = "optionalMods")]
    optional_mods: Vec<RawRepoMod>,
    #[serde(default, rename = "clientParameters")]
    client_parameters: String,
    #[serde(default, rename = "servers")]
    servers: Vec<RawServer>,
    #[serde(default, rename = "repoBasicAuthentication")]
    repo_basic_authentication: Option<RawAuth>,
    #[serde(default)]
    version: String,
}

#[derive(Deserialize)]
struct RawAuth {
    username: String,
    password: String,
}

#[derive(Deserialize)]
struct RawRepoMod {
    #[serde(rename = "modName")]
    mod_name: String,
    #[serde(rename = "checkSum")]
    checksum: crate::digest::Md5Digest,
    enabled: bool,
}

#[derive(Deserialize)]
struct RawServer {
    name: String,
    address: String,
    #[serde(deserialize_with = "de_port")]
    port: u16,
    #[serde(default)]
    password: String,
    #[serde(default, rename = "battleEye")]
    battle_eye: bool,
}

fn de_port<'de, D>(de: D) -> std::result::Result<u16, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct V;
    impl<'de> serde::de::Visitor<'de> for V {
        type Value = u16;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "u16 port as number or string")
        }
        fn visit_u64<E>(self, v: u64) -> std::result::Result<u16, E>
        where
            E: serde::de::Error,
        {
            u16::try_from(v).map_err(|_| E::custom("port out of range"))
        }
        fn visit_str<E>(self, v: &str) -> std::result::Result<u16, E>
        where
            E: serde::de::Error,
        {
            v.parse::<u16>().map_err(|_| E::custom("invalid port"))
        }
    }
    de.deserialize_any(V)
}

pub fn parse_repo_spec(bytes: &[u8]) -> Result<RepoSpec> {
    let raw: RawRepo = serde_json::from_slice(bytes)?;
    Ok(RepoSpec {
        repo_name: raw.repo_name,
        checksum: raw.checksum,
        required_mods: raw
            .required_mods
            .into_iter()
            .map(|m| RepoMod {
                mod_name: m.mod_name,
                checksum: m.checksum,
                enabled: m.enabled,
            })
            .collect(),
        optional_mods: raw
            .optional_mods
            .into_iter()
            .map(|m| RepoMod {
                mod_name: m.mod_name,
                checksum: m.checksum,
                enabled: m.enabled,
            })
            .collect(),
        client_parameters: raw.client_parameters,
        basic_auth: raw.repo_basic_authentication.map(|a| RepoBasicAuth {
            username: a.username,
            password: a.password,
        }),
        version: raw.version,
        servers: raw
            .servers
            .into_iter()
            .map(|s| RepoServer {
                name: s.name,
                address: s.address,
                port: s.port,
                password: s.password,
                battle_eye: s.battle_eye,
            })
            .collect(),
    })
}
