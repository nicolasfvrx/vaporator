use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path, time::Duration};
use steam_vent::{Connection, ConnectionTrait, ServerList};
use steam_vent_proto_steam::steammessages_clientserver_appinfo::{
    CMsgClientPICSAccessTokenRequest, CMsgClientPICSAccessTokenResponse,
    CMsgClientPICSChangesSinceRequest, CMsgClientPICSChangesSinceResponse,
    CMsgClientPICSProductInfoRequest, CMsgClientPICSProductInfoResponse,
    cmsg_client_picsproduct_info_request::AppInfo,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct App {
    pub id: u32,
    pub name: String,
    pub branches: BTreeMap<String, Branch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Branch {
    pub build_id: String,
    pub password_required: bool,
}

pub struct Changes {
    pub current: u32,
    pub full_update: bool,
    pub apps: Vec<u32>,
}

#[derive(Serialize, Deserialize)]
struct Session {
    account: String,
    token: String,
}

#[derive(Clone)]
pub struct Steam(Connection);

impl Steam {
    pub async fn connect(data_dir: &Path) -> Result<Self> {
        tokio::time::timeout(Duration::from_secs(60), async {
            let servers = ServerList::discover()
                .await
                .context("Steam server discovery failed")?;
            let session_path = data_dir.join("steam-session.json");
            let mut connection = if session_path.exists() {
                let session: Session =
                    serde_json::from_slice(&tokio::fs::read(session_path).await?)?;
                Connection::access(&servers, &session.account, &session.token)
                    .await
                    .context("Steam session rejected; run steam-login again")?
            } else {
                Connection::anonymous(&servers)
                    .await
                    .context("Anonymous Steam login failed")?
            };
            connection.set_timeout(Duration::from_secs(30));
            Ok(Self(connection))
        })
        .await
        .context("Steam connection timed out")?
    }

    pub async fn app(&self, id: u32) -> Result<App> {
        let tokens: CMsgClientPICSAccessTokenResponse = self
            .0
            .job(CMsgClientPICSAccessTokenRequest {
                appids: vec![id],
                ..Default::default()
            })
            .await?;
        let token = tokens
            .app_access_tokens
            .iter()
            .find(|t| t.appid() == id)
            .map(|t| t.access_token());
        let response: CMsgClientPICSProductInfoResponse = self
            .0
            .job(CMsgClientPICSProductInfoRequest {
                apps: vec![AppInfo {
                    appid: Some(id),
                    access_token: token,
                    ..Default::default()
                }],
                meta_data_only: Some(false),
                single_response: Some(true),
                ..Default::default()
            })
            .await?;
        let app = response
            .apps
            .iter()
            .find(|a| a.appid() == id)
            .context("Unknown or inaccessible Steam application")?;
        let text = std::str::from_utf8(app.buffer())?.trim_matches('\0').trim();
        parse_app(id, text)
    }

    pub async fn changes(&self, since: u32) -> Result<Changes> {
        let response: CMsgClientPICSChangesSinceResponse = self
            .0
            .job(CMsgClientPICSChangesSinceRequest {
                since_change_number: Some(since),
                send_app_info_changes: Some(true),
                send_package_info_changes: Some(false),
                ..Default::default()
            })
            .await?;
        Ok(Changes {
            current: response.current_change_number(),
            full_update: response.force_full_update() || response.force_full_app_update(),
            apps: response.app_changes.iter().map(|a| a.appid()).collect(),
        })
    }
}

#[derive(Deserialize)]
struct Root {
    appinfo: Info,
}
#[derive(Deserialize)]
struct Info {
    common: Common,
    #[serde(default)]
    depots: Depots,
}
#[derive(Deserialize)]
struct Common {
    name: String,
}
#[derive(Default, Deserialize)]
struct Depots {
    #[serde(default)]
    branches: BTreeMap<String, RawBranch>,
}
#[derive(Deserialize)]
struct RawBranch {
    buildid: Option<String>,
    pwdrequired: Option<String>,
}

fn parse_app(id: u32, text: &str) -> Result<App> {
    let root: Root = vdf_reader::from_str(text).context("Invalid Steam product metadata")?;
    let branches = root
        .appinfo
        .depots
        .branches
        .into_iter()
        .filter_map(|(name, b)| {
            b.buildid.map(|build_id| {
                (
                    name,
                    Branch {
                        build_id,
                        password_required: b.pwdrequired.as_deref() == Some("1"),
                    },
                )
            })
        })
        .collect();
    Ok(App {
        id,
        name: root.appinfo.common.name,
        branches,
    })
}

pub async fn login(data_dir: &Path) -> Result<()> {
    use steam_vent::auth::{
        AuthConfirmationHandler, ConsoleAuthConfirmationHandler, DeviceConfirmationHandler,
    };
    tokio::fs::create_dir_all(data_dir).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(data_dir, std::fs::Permissions::from_mode(0o700)).await?;
    }
    let account = rpassword::prompt_password("Steam account name (hidden): ")?;
    let password = rpassword::prompt_password("Steam password: ")?;
    let servers = ServerList::discover().await?;
    let connection = Connection::login(
        &servers,
        &account,
        &password,
        steam_vent::auth::NullGuardDataStore,
        ConsoleAuthConfirmationHandler::default().or(DeviceConfirmationHandler),
    )
    .await?;
    let Some(token) = connection.access_token() else {
        bail!("Steam did not return a reusable session")
    };
    let session = Session {
        account,
        token: token.to_owned(),
    };
    let path = data_dir.join("steam-session.json");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    serde_json::to_writer(file, &session)?;
    println!("Steam session saved. Protect the data directory and restart the bot.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_branches_and_preserves_unicode() {
        let app = parse_app(42, r#""appinfo" { "common" { "name" "Café" } "depots" { "branches" { "public" { "buildid" "123" } "private" { "buildid" "456" "pwdrequired" "1" } } } }"#).unwrap();
        assert_eq!(app.name, "Café");
        assert_eq!(app.branches["public"].build_id, "123");
        assert!(app.branches["private"].password_required);
    }
}
