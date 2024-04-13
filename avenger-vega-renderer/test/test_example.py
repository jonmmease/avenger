from playwright.sync_api import Page, Playwright
import pytest
from typing import Literal
import json
from pixelmatch.contrib.PIL import pixelmatch
from dataclasses import dataclass
from PIL import Image
from pathlib import Path
import io
import subprocess
import time
import socket

here = Path(__file__).parent


def find_open_port():
    # Create a new socket using the default settings
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        # Bind to localhost on a port chosen by the OS
        s.bind(('localhost', 0))
        # Get the port number that was automatically chosen
        return s.getsockname()[1]


@pytest.fixture(scope="session")
def page_url():
    port = find_open_port()
    cwd = here / "test_server"
    # Run npm install
    subprocess.call(["pixi", "run", "npm", "install"], cwd=cwd)

    # Launch dev web server
    p = subprocess.Popen(["./node_modules/.bin/webpack-dev-server",  "--port",  str(port)], cwd=cwd)

    time.sleep(2)
    yield f"http://localhost:{port}/"
    p.terminate()


def test_screenshot(playwright: Playwright, page_url):
    chromium = playwright.chromium
    browser = chromium.launch(
        headless=True,
    )
    page = browser.new_page()
    print(page_url)
    page.goto(page_url)
    page.wait_for_timeout(1000)

    spec = spec1()
    comparison_res = compare(page, spec)

    comparison_res.canvas_img.save("img_canvas.png")
    comparison_res.avenger_img.save("img_avenger.png")
    comparison_res.diff_img.save("img_diff.png")
    print(comparison_res)


@dataclass
class ComparisonResult:
    canvas_img: Image
    avenger_img: Image
    diff_img: Image
    mistmatch: int
    score: float


def compare(page: Page, spec: dict) -> ComparisonResult:
    canvas_img = spec_to_image(page, spec, "canvas")
    avenger_img = spec_to_image(page, spec, "avenger")
    diff_img = Image.new("RGBA", canvas_img.size)
    mismatch = pixelmatch(canvas_img, avenger_img, diff_img, threshold=0.2)
    score = mismatch / (canvas_img.width * canvas_img.height)
    return ComparisonResult(
        canvas_img=canvas_img,
        avenger_img=avenger_img,
        diff_img=diff_img,
        mistmatch=mismatch,
        score=score
    )


def spec_to_image(page: Page, spec: dict, renderer: Literal["canvas", "avenger"]) -> Image:
    embed_opts = {
        "actions": False,
        "renderer": renderer
    }
    script = f"vegaEmbed('#plot-container', {json.dumps(spec)}, {json.dumps(embed_opts)});"
    page.evaluate_handle(script)
    return Image.open(io.BytesIO(page.locator("canvas").first.screenshot()))


def spec1():
    return json.loads(r"""
{
  "$schema": "https://vega.github.io/schema/vega/v5.json",
  "description": "A basic scatter plot example depicting automobile statistics.",
  "width": 200,
  "height": 200,
  "padding": 5,

  "data": [
    {
      "name": "source",
      "url": "https://raw.githubusercontent.com/vega/vega-datasets/main/data/cars.json",
      "transform": [
        {
          "type": "filter",
          "expr": "datum['Horsepower'] != null && datum['Miles_per_Gallon'] != null && datum['Acceleration'] != null"
        }
      ]
    }
  ],

  "scales": [
    {
      "name": "x",
      "type": "linear",
      "round": true,
      "nice": true,
      "zero": true,
      "domain": {"data": "source", "field": "Horsepower"},
      "range": "width"
    },
    {
      "name": "y",
      "type": "linear",
      "round": true,
      "nice": true,
      "zero": true,
      "domain": {"data": "source", "field": "Miles_per_Gallon"},
      "range": "height"
    },
    {
      "name": "size",
      "type": "linear",
      "round": true,
      "nice": false,
      "zero": true,
      "domain": {"data": "source", "field": "Acceleration"},
      "range": [4,361]
    }
  ],

  "axes": [
    {
      "scale": "x",
      "grid": true,
      "domain": false,
      "orient": "bottom",
      "tickCount": 5,
      "title": "Horsepower"
    },
    {
      "scale": "y",
      "grid": true,
      "domain": false,
      "orient": "left",
      "titlePadding": 5,
      "title": "Miles_per_Gallon"
    }
  ],

  "legends": [
    {
      "size": "size",
      "title": "Accelerate",
      "symbolOpacity": 0.5,
      "symbolStrokeWidth": 0,
      "symbolFillColor": "grey",
      "symbolType": "circle"
    }
  ],

  "marks": [
    {
      "name": "marks",
      "type": "symbol",
      "from": {"data": "source"},
      "encode": {
        "update": {
          "x": {"scale": "x", "field": "Horsepower"},
          "y": {"scale": "y", "field": "Miles_per_Gallon"},
          "size": {"scale": "size", "field": "Acceleration"},
          "shape": {"value": "circle"},
          "opacity": {"value": 0.5},
          "fill": {"value": "gray"}
        }
      }
    }
  ]
}    
    """)
