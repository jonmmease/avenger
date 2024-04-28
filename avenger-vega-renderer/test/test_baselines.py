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
import shutil

here = Path(__file__).parent
specs_root = here.parent.parent / "avenger-vega-test-data" / "vega-specs"


def find_open_port():
    # Create a new socket using the default settings
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as s:
        # Bind to localhost on a port chosen by the OS
        s.bind(("localhost", 0))
        # Get the port number that was automatically chosen
        return s.getsockname()[1]


@pytest.fixture(scope="session")
def page_url():
    port = find_open_port()
    cwd = here / "test_server"
    # Run npm install
    subprocess.call(["pixi", "run", "npm", "install"], cwd=cwd)

    # Launch dev web server
    p = subprocess.Popen(
        ["./node_modules/.bin/webpack-dev-server", "--port", str(port)], cwd=cwd
    )

    time.sleep(2)
    yield f"http://localhost:{port}/"
    p.terminate()


@pytest.fixture(scope="session")
def failures_path():
    path = here / "failures"
    shutil.rmtree(path, ignore_errors=True)
    path.mkdir(parents=True, exist_ok=True)
    return path


@pytest.mark.parametrize(
    "category,spec_name,tolerance",
    [
        ("rect", "stacked_bar", 0.0001),
        ("rect", "stacked_bar_stroke", 0.0001),
        ("rect", "stacked_bar_rounded", 0.0001),
        ("rect", "stacked_bar_rounded_stroke", 0.0001),
        ("rect", "stacked_bar_rounded_stroke_opacity", 0.0001),
        ("rect", "heatmap", 0.0001),

        ("symbol", "binned_scatter_diamonds", 0.0001),
        ("symbol", "binned_scatter_square", 0.0001),
        ("symbol", "binned_scatter_triangle-down", 0.0001),
        ("symbol", "binned_scatter_triangle-up", 0.0001),
        ("symbol", "binned_scatter_triangle-left", 0.0001),
        ("symbol", "binned_scatter_triangle-right", 0.0001),
        ("symbol", "binned_scatter_triangle", 0.0001),
        ("symbol", "binned_scatter_wedge", 0.0003),
        ("symbol", "binned_scatter_arrow", 0.0001),
        ("symbol", "binned_scatter_cross", 0.0001),
        ("symbol", "binned_scatter_circle", 0.0001),
        ("symbol", "binned_scatter_path", 0.0001),
        ("symbol", "binned_scatter_path_star", 0.0001),
        ("symbol", "binned_scatter_path_star", 0.0001),
        ("symbol", "binned_scatter_cross_stroke", 0.0001),
        ("symbol", "binned_scatter_circle_stroke", 0.0001),
        ("symbol", "binned_scatter_circle_stroke_no_fill", 0.0001),
        ("symbol", "binned_scatter_path_star_stroke_no_fill", 0.0001),
        ("symbol", "scatter_transparent_stroke", 0.0001),
        ("symbol", "scatter_transparent_stroke_star", 0.0001),
        ("symbol", "scatter_transparent_stroke_star", 0.0001),
        ("symbol", "wind_vector", 0.0001),
        ("symbol", "wedge_angle", 0.0001),
        ("symbol", "wedge_stroke_angle", 0.0001),
        ("symbol", "zindex_circles", 0.0001),
        ("symbol", "mixed_symbols", 0.0001),

        # lyon seems to omit closing square cap, need to investigate
        ("rule", "wide_transparent_caps", 0.003),
        ("rule", "dashed_rules", 0.0004),
        ("rule", "wide_rule_axes", 0.0001),

        ("text", "text_alignment", 0.016),
        ("text", "text_rotation", 0.016),
        ("text", "letter_scatter", 0.027),
        ("text", "lasagna_plot", 0.04),
        ("text", "arc_radial", 0.005),
        ("text", "emoji", 0.05),

        ("arc", "single_arc_no_inner", 0.0001),
        ("arc", "single_arc_with_inner_radius", 0.0001),
        ("arc", "single_arc_with_inner_radius_wrap", 0.0001),
        ("arc", "single_arc_with_inner_radius_wrap_stroke", 0.0001),
        ("arc", "arcs_with_variable_outer_radius", 0.0001),
        ("arc", "arcs_with_variable_outer_radius_stroke", 0.0001),
        ("arc", "arc_with_stroke", 0.0001),

        ("path", "single_path_no_stroke", 0.0),
        ("path", "multi_path_no_stroke", 0.0),
        ("path", "single_path_with_stroke", 0.0),
        ("path", "single_path_with_stroke_no_fill", 0.0),
        ("path", "multi_path_with_stroke", 0.0),
        ("path", "multi_path_with_stroke_no_fill", 0.0),
        ("shape", "us-counties", 0.0001),
        ("shape", "us-map", 0.0001),
        ("shape", "world-natural-earth-projection", 0.0001),
        ("shape", "london_tubes", 0.0001),

        ("line", "simple_line_round_cap", 0.0),
        ("line", "simple_line_butt_cap_miter_join", 0.0),
        ("line", "simple_line_square_cap_bevel_join", 0.0002),  # square-cap
        ("line", "connected_scatter", 0.0001),
        ("line", "lines_with_open_symbols", 0.0),
        ("line", "stocks", 0.0001),
        ("line", "simple_dashed", 0.0),
        ("line", "line_dashed_round_undefined", 0.0),
        ("line", "line_dashed_square_undefined", 0.02),  # square-cap
        ("line", "line_dashed_butt_undefined", 0.0),
        ("line", "stocks-legend", 0.006),
        ("line", "stocks_dashed", 0.006),

        ("area", "100_percent_stacked_area", 0.0),
        ("area", "simple_unemployment", 0.0),
        ("area", "simple_unemployment_stroke", 0.0),
        ("area", "stacked_area", 0.0001),
        ("area", "streamgraph_area", 0.0002),
        ("area", "with_undefined", 0.0),
        ("area", "with_undefined_horizontal", 0.0),

        ("trail", "trail_stocks", 0.0),
        ("trail", "trail_stocks_opacity", 0.0),

        ("image", "logos", 0.0002),
        ("image", "logos_sized_aspect_false", 0.0002),
        ("image", "logos_sized_aspect_false_align_baseline", 0.0002),
        ("image", "logos_sized_aspect_true_align_baseline", 0.0002),
        # ("image", "smooth_false", 0.0),   # Smooth false not supported yet
        ("image", "smooth_true", 0.0002),
        ("image", "many_images", 0.04),     # svg renderer shows missing images for some
        ("image", "large_images", 0.0002),   # CORS issue loading from cdn

        ("gradients", "symbol_cross_gradient", 0.0001),
        ("gradients", "symbol_circles_gradient_stroke", 0.0001),
        ("gradients", "symbol_radial_gradient", 0.0002),
        ("gradients", "rules_with_gradients", 0.003),  # Lyon square caps issue
        ("gradients", "heatmap_with_colorbar", 0.0008),
        ("gradients", "diagonal_gradient_bars_rounded", 0.0001),
        ("gradients", "default_gradient_bars_rounded_stroke", 0.0001),
        ("gradients", "residuals_colorscale", 0.001),
        ("gradients", "stroke_rect_gradient", 0.0001),
        ("gradients", "arc_gradient", 0.0001),
        ("gradients", "path_with_stroke_gradients", 0.0),

        ("clip", "text_clip", 0.006),
        ("clip", "text_clip_rounded", 0.006),
        ("clip", "bar_rounded2", 0.0),
        ("clip", "clip_mixed_marks", 0.0),
        ("clip", "clip_rounded", 0.0),
    ],
)
def test_image_baselines(
    playwright: Playwright, page_url, failures_path, category, spec_name, tolerance
):
    chromium = playwright.chromium
    browser = chromium.launch(
        headless=True,
    )
    page = browser.new_page()
    page.goto(page_url)
    page.wait_for_timeout(1000)

    spec_path = specs_root / category / (spec_name + ".vg.json")
    with open(spec_path, "rt") as f:
        spec = json.load(f)

    comparison_res = compare(page, spec)
    page.close()
    print(f"score: {comparison_res.score}")
    if comparison_res.score > tolerance:
        outdir = failures_path / category / spec_name
        outdir.mkdir(parents=True, exist_ok=True)
        comparison_res.svg_img.save(outdir / "svg.png")
        comparison_res.avenger_img.save(outdir / "avenger.png")
        comparison_res.diff_img.save(outdir / "diff.png")
        with open(outdir / f"metrics.json", "wt") as f:
            json.dump(
                {
                    "required_score": tolerance,
                    "score": comparison_res.score,
                    "mismatch": comparison_res.mismatch,
                },
                f,
                indent=2,
            )
        assert comparison_res.score <= tolerance


@dataclass
class ComparisonResult:
    svg_img: Image
    avenger_img: Image
    diff_img: Image
    mismatch: int
    score: float


def compare(page: Page, spec: dict) -> ComparisonResult:
    avenger_errs = []
    page.on("pageerror", lambda e: avenger_errs.append(e))
    avenger_img = spec_to_image(page, spec, "avenger")
    if avenger_errs:
        pytest.fail(avenger_errs)

    svg_img = spec_to_image(page, spec, "svg")
    diff_img = Image.new("RGBA", svg_img.size)
    mismatch = pixelmatch(svg_img, avenger_img, diff_img, threshold=0.2)
    score = mismatch / (svg_img.width * svg_img.height)
    return ComparisonResult(
        svg_img=svg_img,
        avenger_img=avenger_img,
        diff_img=diff_img,
        mismatch=mismatch,
        score=score,
    )


def spec_to_image(
    page: Page, spec: dict, renderer: Literal["canvas", "avenger", "svg"]
) -> Image:
    embed_opts = {"actions": False, "renderer": renderer}
    script = (
        f"vegaEmbed('#plot-container', {json.dumps(spec)}, {json.dumps(embed_opts)});"
    )
    page.evaluate_handle(script)
    page.wait_for_timeout(1000)
    if renderer == "svg":
        locator = page.locator("svg")
    else:
        locator = page.locator("canvas")
    img = Image.open(io.BytesIO(locator.first.screenshot()))

    # Check that the image is not entirely white (which happens on rendering errors sometimes)
    pixels = img.load()
    for x in range(img.width):
        for y in range(img.height):
            if pixels[x, y] != (255, 255, 255, 255):
                # Found non-white pixel, return
                return img

    pytest.fail("Retrieved blank image")
