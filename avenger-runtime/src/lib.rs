pub mod value;
pub mod error;
pub mod memory;
pub mod variable;
pub mod cache;
pub mod task_graph;
pub mod tasks;
pub mod dependency;
pub mod runtime;
pub mod context;
pub mod marks;
pub mod component_registry;
pub mod scope;


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
