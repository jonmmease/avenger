// The chart format in TypeScript: the types and schemas that protobuf-es generates from
// `avenger-chart/proto`, and the decoding and encoding of chart messages. Tables and values stay
// Arrow IPC streams, which a library such as `apache-arrow` decodes.

import { fromBinary, toBinary } from "@bufbuild/protobuf";
import { type Chart, ChartSchema } from "./gen/avenger/chart/v1/chart_pb.js";

export * from "./gen/avenger/chart/v1/annotation_pb.js";
export * from "./gen/avenger/chart/v1/attribution_pb.js";
export * from "./gen/avenger/chart/v1/axis_pb.js";
export * from "./gen/avenger/chart/v1/cartesian_pb.js";
export * from "./gen/avenger/chart/v1/channel_pb.js";
export * from "./gen/avenger/chart/v1/chart_pb.js";
export * from "./gen/avenger/chart/v1/element_pb.js";
export * from "./gen/avenger/chart/v1/encoding_pb.js";
export * from "./gen/avenger/chart/v1/facet_pb.js";
export * from "./gen/avenger/chart/v1/geo_pb.js";
export * from "./gen/avenger/chart/v1/legend_pb.js";
export * from "./gen/avenger/chart/v1/mark_pb.js";
export * from "./gen/avenger/chart/v1/option_pb.js";
export * from "./gen/avenger/chart/v1/part_pb.js";
export * from "./gen/avenger/chart/v1/partition_pb.js";
export * from "./gen/avenger/chart/v1/plot_pb.js";
export * from "./gen/avenger/chart/v1/polar_pb.js";
export * from "./gen/avenger/chart/v1/resolve_pb.js";
export * from "./gen/avenger/chart/v1/route_pb.js";
export * from "./gen/avenger/chart/v1/scale_pb.js";
export * from "./gen/avenger/chart/v1/selection_pb.js";
export * from "./gen/avenger/chart/v1/side_pb.js";
export * from "./gen/avenger/chart/v1/style_pb.js";
export * from "./gen/avenger/chart/v1/tool_pb.js";
export * from "./gen/avenger/chart/v1/value_pb.js";
export * from "./gen/avenger/chart/v1/widget_pb.js";

/** Reads a chart message. The bindings do not check the chart's rules, which the reader of the library does. */
export function decodeChart(bytes: Uint8Array): Chart {
  return fromBinary(ChartSchema, bytes);
}

/** Writes a chart message. */
export function encodeChart(chart: Chart): Uint8Array {
  return toBinary(ChartSchema, chart);
}
