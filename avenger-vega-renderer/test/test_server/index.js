import vegaEmbed from 'vega-embed';
import { registerVegaRenderer } from 'avenger-vega-renderer';
import { renderModule } from 'vega-scenegraph';

// Simple initial chart spec that will be replaced using playwright
const spec = {
    "$schema": "https://vega.github.io/schema/vega/v5.json",
    "description": "A basic stacked bar chart example.",
    "width": 200,
    "height": 200,
    "padding": 5,
    "title": "Avenger test harness",
    "background": "lightgray",
};

// Make the "avenger" renderer available
registerVegaRenderer(renderModule, true);

// Make vega embed available globally so that we can call it using playwright
window.vegaEmbed = vegaEmbed;
vegaEmbed('#plot-container', spec, {
    renderer: "canvas",
}).catch(console.error);
