pub mod ast;
pub mod parser;
pub mod error;
pub mod visitor;
pub mod interpreter;
pub mod environment;
pub mod udtf;
mod table;

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
