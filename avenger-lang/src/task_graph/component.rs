use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PropType {
    Val,
    Expr,
    Dataset,
}


#[derive(Clone, Debug)]
pub struct ComponentSpec {
    pub name: String,
    pub props: HashMap<String, PropType>,
    pub allow_children: bool,
    pub is_mark: bool,
}

impl ComponentSpec {
    pub fn new(name: String, props: HashMap<String, PropType>, allow_children: bool, is_mark: bool) -> Self {
        Self { name, props, allow_children, is_mark  }
    }    
}


// Should default values be stored here?
// =====================================
// pub enum PropSpec {
//     Val {
//         default: Option<SqlExpr>,
//     },
//     Expr {
//         default: Option<SqlExpr>,
//     },
//     Dataset {
//         default: Option<SqlQuery>,
//     },
// }
// 
// impl PropSpec {
//     pub fn val(default: SqlExpr) -> Self {
//         Self::Val { default: Some(default) }
//     }
// 
//     pub fn val_required() -> Self {
//         Self::Val { default: None }
//     }
// 
//     pub fn expr(default: SqlExpr) -> Self {
//         Self::Expr { default: Some(default) }
//     }
// 
//     pub fn expr_required() -> Self {
//         Self::Expr { default: None }
//     }
// 
//     pub fn dataset(default: SqlQuery) -> Self {
//         Self::Dataset { default: Some(default) }
//     }
// 
//     pub fn dataset_required() -> Self {
//         Self::Dataset { default: None }
//     }
// }
// =====================================
