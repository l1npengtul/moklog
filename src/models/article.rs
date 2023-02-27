use sea_orm::DeriveEntityModel;

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
#[sea_orm(table_name = "articles")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub hash: i64,
    pub original_path: String,
    
}

pub enum ArticleType {
    ArticleBuild,
    ArticlePrebuilt,
}