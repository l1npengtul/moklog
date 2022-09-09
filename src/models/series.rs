use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "series")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: u64,
    pub start_date: DateTimeWithTimeZone,
    pub completed_date: Option<DateTimeWithTimeZone>,
    pub tags: Json,
    pub category: String,
    pub author: String,
    pub title: String,
    pub slug: String,
    pub pages: Json, // Vec<u64>
}

#[derive(Copy, Clone, Debug, Ord, PartialOrd, Eq, PartialEq, EnumIter)]
pub enum Relation {}

impl RelationTrait for Relation {
    fn def(&self) -> RelationDef {
        todo!()
    }
}

impl ActiveModelBehavior for ActiveModel {}
