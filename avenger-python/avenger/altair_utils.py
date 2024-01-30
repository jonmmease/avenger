from ._avenger import SceneGraph


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
