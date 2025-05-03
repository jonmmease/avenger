pub mod cache;
pub mod component_registry;
pub mod context;
pub mod dependency;
pub mod error;
pub mod marks;
pub mod memory;
pub mod runtime;
pub mod scope;
pub mod task_graph;
pub mod tasks;
pub mod udtf;
pub mod value;
pub mod variable;

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
