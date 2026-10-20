//! Settings → Branding: the panel's name, primary color, logos and favicon,
//! changed while it runs (RustCMS's runtime branding: [`PanelBranding`] is
//! its source). Kept in `panel_settings` (name, color) and
//! `branding_images`; checked with the rules the panel serves them by.
//! The panel's administrators' (`system.manage`).

use base64::Engine;
use rustcms::admin::{AdminBranding, Branding, BrandingSource};
use rustcms::{AppError, AppResult, AppState, ValidationErrors, async_trait};
use sea_orm::{ConnectionTrait, EntityTrait, Set};
use serde_json::{Value, json};

use crate::models::{branding_image, panel_setting};

const NAME_KEY: &str = "branding.name";
const PRIMARY_KEY: &str = "branding.primary";
/// The images, by kind.
pub const KINDS: [&str; 3] = ["logo", "logo-dark", "favicon"];

/// Where the panel's branding comes from: what Settings → Branding keeps
/// (nothing set: the static brand and theme).
pub struct PanelBranding;

#[async_trait]
impl BrandingSource for PanelBranding {
    async fn load(&self, app: &AppState) -> AppResult<Option<Branding>> {
        let db = app.db();
        let name = panel_setting::get(db, NAME_KEY).await?;
        let primary = panel_setting::get(db, PRIMARY_KEY).await?;
        let images = branding_image::Entity::find().all(db).await?;
        if name.is_none() && primary.is_none() && images.is_empty() {
            return Ok(None);
        }
        // Checked when saved: one refused now (the rules changed) is a
        // configuration error, and the static theme shows.
        let invalid = |e: rustcms::admin::BrandingError| AppError::internal(e.to_string());
        let mut branding = Branding::new();
        if let Some(name) = name {
            branding = branding.name(&name).map_err(invalid)?;
        }
        if let Some(primary) = primary {
            branding = branding.primary(&primary).map_err(invalid)?;
        }
        for image in images {
            branding = match image.kind.as_str() {
                "logo" => branding.logo(image.data, &image.mime),
                "logo-dark" => branding.logo_dark(image.data, &image.mime),
                "favicon" => branding.favicon(image.data, &image.mime),
                _ => Ok(branding),
            }
            .map_err(invalid)?;
        }
        Ok(Some(branding))
    }
}

/// What Settings shows: the name, the color and each image (a data URL to
/// preview it).
pub async fn current<C: ConnectionTrait>(db: &C) -> AppResult<Value> {
    let mut images = serde_json::Map::new();
    for image in branding_image::Entity::find().all(db).await? {
        let data = base64::engine::general_purpose::STANDARD.encode(&image.data);
        images.insert(
            image.kind.clone(),
            json!({
                "mime": image.mime,
                "size": image.data.len(),
                "url": format!("data:{};base64,{data}", image.mime),
            }),
        );
    }
    Ok(json!({
        "name": panel_setting::get(db, NAME_KEY).await?,
        "primary": panel_setting::get(db, PRIMARY_KEY).await?,
        "images": images,
        "kinds": KINDS,
    }))
}

/// Sets the name and the primary color (empty or none: the static one).
pub async fn save<C: ConnectionTrait>(
    db: &C,
    name: Option<&str>,
    primary: Option<&str>,
) -> AppResult<()> {
    let name = name.map(str::trim).filter(|n| !n.is_empty());
    let primary = primary.map(str::trim).filter(|p| !p.is_empty());
    let mut errors = ValidationErrors::new();
    if let Some(name) = name
        && let Err(e) = Branding::check_name(name)
    {
        errors.add("name", e.to_string());
    }
    if let Some(primary) = primary
        && let Err(e) = Branding::check_color("primary", primary)
    {
        errors.add("primary", e.to_string());
    }
    if !errors.is_empty() {
        return Err(errors.into());
    }
    for (key, value) in [(NAME_KEY, name), (PRIMARY_KEY, primary)] {
        match value {
            Some(value) => panel_setting::set(db, key, value).await?,
            None => {
                panel_setting::Entity::delete_by_id(key.to_owned())
                    .exec(db)
                    .await?;
            }
        }
    }
    Ok(())
}

/// Keeps `bytes` as the image `kind` (PNG, WebP or ICO up to 512 KiB).
pub async fn set_image<C: ConnectionTrait>(
    db: &C,
    kind: &str,
    mime: &str,
    bytes: Vec<u8>,
) -> AppResult<()> {
    if !KINDS.contains(&kind) {
        return Err(AppError::NotFound);
    }
    Branding::check_image(&bytes, mime)
        .map_err(|e| ValidationErrors::field("data", e.to_string()))?;
    use sea_orm::sea_query::OnConflict;
    branding_image::Entity::insert(branding_image::ActiveModel {
        kind: Set(kind.to_owned()),
        mime: Set(mime.to_owned()),
        data: Set(bytes),
        updated_at: Set(chrono::Utc::now()),
    })
    .on_conflict(
        OnConflict::column(branding_image::Column::Kind)
            .update_columns([
                branding_image::Column::Mime,
                branding_image::Column::Data,
                branding_image::Column::UpdatedAt,
            ])
            .to_owned(),
    )
    .exec_without_returning(db)
    .await?;
    Ok(())
}

/// Removes the image `kind` (the static one shows again).
pub async fn remove_image<C: ConnectionTrait>(db: &C, kind: &str) -> AppResult<()> {
    if !KINDS.contains(&kind) {
        return Err(AppError::NotFound);
    }
    branding_image::Entity::delete_by_id(kind.to_owned())
        .exec(db)
        .await?;
    Ok(())
}

/// This instance shows the change on its next request (others within
/// `[admin] branding_ttl_secs`).
pub fn invalidate(app: &AppState) {
    if let Some(branding) = app.service::<AdminBranding>() {
        branding.invalidate();
    }
}
