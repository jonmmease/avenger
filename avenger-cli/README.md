# Avenger CLI

This is a CLI application for the Avenger visualization language.

## Usage

Run the application with:

```bash
cargo run preview path/to/my-visualization.avgr
```

By default, the application will create and display the visualization defined in `app.avgr` in the current directory.

You can specify a different file path:

```bash
cargo run preview path/to/my-visualization.avgr
```

If the specified file doesn't exist, the application will create it with a default visualization.

## Features

- **Live Preview**: The application automatically watches for changes to the `.avgr` file and updates the visualization in real-time.
- **Error Handling**: If there are errors in your Avenger code, they will be displayed in the console without crashing the application.
- **Simple Interface**: Focus on writing your Avenger code, and see the results immediately.
