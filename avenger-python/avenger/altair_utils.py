from ._avenger import SceneGraph
import copy

def avenger_png_renderer(spec: dict, **kwargs) -> dict:
    """
    Altair renderer plugin that uses Avenger to render charts to PNG

    This function is registered as avenger-png in the altair.vegalite.v5.renderer
    entry point group. It may be enabled in Altair using:

        >>> import altair as alt
        >>> alt.renderers.enable('avenger-png')

    See https://altair-viz.github.io/user_guide/custom_renderers.html
    for more information
    """
    import altair as alt
    import vl_convert as vlc

    if alt.data_transformers.active == "vegafusion":
        # When the vegafusion transformer is enabled we convert the spec to
        # Vega, which will include the pre-transformed inline data
        vg_spec = alt.Chart.from_dict(spec).to_dict(format="vega")
        vega_sg = vlc.vega_to_scenegraph(vg_spec)
    else:
        vega_sg = vlc.vegalite_to_scenegraph(spec)

    sg = SceneGraph.from_vega_scenegraph(vega_sg)
    return {"image/png": sg.to_png(scale=kwargs.get("scale", None))}


def avenger_html_renderer(spec: dict, verbose=False, **kwargs) -> dict:
    """
    Altair renderer plugin that uses Avenger to render interactive charts

    This function is registered as avenger-html in the altair.vegalite.v5.renderer
    entry point group. It may be enabled in Altair using:

        >>> import altair as alt
        >>> alt.renderers.enable('avenger-html')

    See https://altair-viz.github.io/user_guide/custom_renderers.html
    for more information
    """
    from altair.utils.mimebundle import spec_to_mimebundle
    from altair import VEGA_VERSION, VEGALITE_VERSION, VEGAEMBED_VERSION
    import jinja2

    template = jinja2.Template(
        """\
<!DOCTYPE html>
<html>
<head>
  <style>
    #{{ output_div }}.vega-embed {
      width: 100%;
      display: flex;
    }

    #{{ output_div }}.vega-embed details,
    #{{ output_div }}.vega-embed details summary {
      position: relative;
    }
  </style>
</head>
<body>
<div class="vega-visualization" id="{{ output_div }}"></div>
<script type="module">
    import vegaEmbed, { vega } from "https://esm.sh/vega-embed@6?deps=vega@5&deps=vega-lite@5.17.0";
    import { registerVegaRenderer } from "https://esm.sh/avenger-vega-renderer@0.0.4";
    registerVegaRenderer(vega.renderModule, """ + str(verbose).lower() + """);
    
    const spec = {{ spec }};
    const embedOpt = {{ embed_options }};
    vegaEmbed('#{{ output_div }}', spec, embedOpt).catch(console.error);
</script>
</body>
</html>
"""
    )

    embed_options = copy.deepcopy(kwargs.get("embed_options", {}))
    embed_options["renderer"] = "avenger"
    bundle = spec_to_mimebundle(
        spec,
        format="html",
        mode="vega-lite",
        template=template,
        embed_options=embed_options,
        vega_version=VEGA_VERSION,
        vegaembed_version=VEGAEMBED_VERSION,
        vegalite_version=VEGALITE_VERSION,
    )
    return bundle


def chart_to_png(chart, scale=1) -> bytes:
    """
    Convert an altair chart to a png image bytes
    :param chart: Altair Chart
    :param scale: Scale factor (default 1.0)
    :return: png image bytes
    """
    import altair as alt
    import vl_convert as vlc
    if alt.data_transformers.active == "vegafusion":
        vg_spec = chart.to_dict(format="vega")
        vega_sg = vlc.vega_to_scenegraph(vg_spec)
    else:
        vl_spec = chart.to_dict(format="vega-lite")
        vega_sg = vlc.vegalite_to_scenegraph(vl_spec)
    sg = SceneGraph.from_vega_scenegraph(vega_sg)
    return sg.to_png(scale=scale)
