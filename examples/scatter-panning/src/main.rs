use crate::util::run;

mod util;

fn main() {
    pollster::block_on(run());
}
