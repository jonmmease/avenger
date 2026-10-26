# movie-search

A plot of movies' Rotten Tomatoes ratings against their IMDB ratings, with a
search box above it. As you type in the box, the plot highlights the movies
whose titles contain the typed text, ignoring case, and the heading counts
them.

- Click the box, or press Tab, to type in it. Arrow keys move the caret, Shift
  extends the selection, and Cmd+C, Cmd+X and Cmd+V, or Ctrl with the same
  keys, copy, cut and paste. The avenger-widgets README lists every key.
- Drag across the box's text to select it. Double-click a word to select the
  word, and drag after the double-click to select whole words.
- Hover near a movie for a tooltip of its title, ratings, genre and year.
- Drag across the heading, the axis labels or the box's label to select their
  text, and copy it with Cmd+C or Ctrl+C.

App state holds the query and whether each movie matches it. The search box is
a `TextInput` from avenger-widgets, whose `on_change` stores the query, matches
it against the lowercase titles, and rebuilds the chart and its hit-testing
tree. The chart draws the movies that do not match as small gray points, and
the matches above them as larger blue points. Without a query, every movie
matches and draws as a small blue point. Each mark's rows go with the scene, so
the `Tooltips` tool from avenger-app-tools describes the movie nearest the
pointer from its row.

## Data

`data/movies.csv` holds the 2,259 movies in the vega-datasets
[`movies.json`](https://raw.githubusercontent.com/vega/vega-datasets/main/data/movies.json)
that have a title, an IMDB rating and a Rotten Tomatoes rating. It keeps their
title, both ratings, major genre and release year, in the source's order. The
source dates 13 films from before 1950 a century late, such as Gone with the
Wind in 2039, so the conversion moves years after 2010 back a century. Both
targets embed the file in the binary. The file is a Git LFS object, so if it is
still a pointer file, run `git lfs pull`.

## Run the example

Run the example natively:

```sh
cargo run --release -p movie-search
```

Or build the example for the web, serve this directory, and open
`http://localhost:8775`:

```sh
wasm-pack build --target web --release
python3 -m http.server 8775
```
