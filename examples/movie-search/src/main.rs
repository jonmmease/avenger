fn main() {
    pollster::block_on(movie_search::run());
}
