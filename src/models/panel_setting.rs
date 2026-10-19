//! `panel_settings` table: the panel's own settings changed in its
//! interface, one value per key.

use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "panel_settings")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub key: String,
    #[sea_orm(column_type = "Text")]
    pub value: String,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

/// The value of `key` (`None`: never set).
pub async fn get<C: ConnectionTrait>(db: &C, key: &str) -> Result<Option<String>, DbErr> {
    Ok(Entity::find_by_id(key.to_owned())
        .one(db)
        .await?
        .map(|row| row.value))
}

/// Sets `key` to `value` (one statement: two first saves never collide).
pub async fn set<C: ConnectionTrait>(db: &C, key: &str, value: &str) -> Result<(), DbErr> {
    use sea_orm::Set;
    use sea_orm::sea_query::OnConflict;
    Entity::insert(ActiveModel {
        key: Set(key.to_owned()),
        value: Set(value.to_owned()),
        updated_at: Set(chrono::Utc::now()),
    })
    .on_conflict(
        OnConflict::column(Column::Key)
            .update_columns([Column::Value, Column::UpdatedAt])
            .to_owned(),
    )
    .exec_without_returning(db)
    .await?;
    Ok(())
}
