mod util;
use crate::util::run;


fn main() {
    pollster::block_on(run());
}
