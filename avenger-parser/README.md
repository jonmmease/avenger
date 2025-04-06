# Avenger Parser

A parser for the Avenger language, a domain-specific language for creating reactive visualizations.

## Features

The Avenger parser supports the following language features:

### Import Statements

Import components from other Avenger files:

```
import { Button } from './components/button.avgr';
import { Header, Footer } from './layout';
```

### Enum Definitions

Define enums for use in components:

```
enum CardSuit { 'clubs', 'diamonds', 'hearts', 'spades' }
export enum Status { 'pending', 'active', 'completed', 'failed' }
```

### Component Declarations

Define components with properties, parameters, and nested components:

```
Chart {
    // Properties
    width: 100;
    height: 100;

    // Parameters
    param<Number> scale: 1.5;
    in param<String> title: "Chart Title" = "Default Title";
    out param<Boolean> interactive: true;

    // Nested components
    Rule {
        x1: 0;
        y1: 0;
        x2: 100;
        y2: 100;
    }
}
```

### Component Inheritance

Components can inherit from other components:

```
LineChart: Chart {
    renderer: "canvas";
}
```

### Component Bindings

Create bindings to components:

```
legend := Legend {
    orientation: "vertical";
    title: "Side Legend";
}
```

### Conditional Logic

Use if statements to conditionally include components or properties:

```
if interactive {
    Legend {
        orientation: "horizontal";
        title: "Chart Legend";
    }
}

if not hideaxis {
    Axis {
        orientation: "bottom";
    }
}
```

### Match Statements

Use match statements for more complex conditional logic:

```
match charttype {
    'bar' => {
        Bar {
            x: "category";
            y: "value";
            fill: "blue";
        }
    }
    'line' => {
        Line {
            x: "category";
            y: "value";
            stroke: "red";
        }
    }
    '_' => {
        Text {
            text: "Unsupported chart type";
            x: 50;
            y: 50;
        }
    }
}
```

### Export Qualifier

Components and enums can be exported for use in other files:

```
export component Button {
    width: 100;
    height: 30;
}

export enum Theme { 'light', 'dark', 'system' }
```

## Building and Testing

Build the parser:

```
cargo build
```

Run tests:

```
cargo test
```

Parse an Avenger file:

```
cargo run -- path/to/file.avgr
```

## Examples

See the `examples` directory for sample Avenger files demonstrating all language features.
