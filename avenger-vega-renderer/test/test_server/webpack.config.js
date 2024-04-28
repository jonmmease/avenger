const CopyWebpackPlugin = require("copy-webpack-plugin");
const path = require('path');

module.exports = {
  entry: "./bootstrap.js",
  output: {
    path: path.resolve(__dirname, "dist"),
    filename: "bootstrap.js",
  },
  mode: "development",
  devtool: 'source-map', // Enables source maps
  experiments: {
    asyncWebAssembly: true,  // enabling async WebAssembly
  },
  module: {
    rules: [
      {
        test: /\.wasm$/,
        type: "webassembly/async",
      },
    ],
  },
  plugins: [
    new CopyWebpackPlugin({
      patterns: [
        { from: 'index.html', to: 'index.html' },
      ],
    }),
  ],
  devServer: {
    client: {
      overlay: false,  // Disabling the error overlay
    },
    headers: {
      'Access-Control-Allow-Origin': '*', // Allows access from any origin
      'Access-Control-Allow-Methods': 'GET, POST, PUT, DELETE, PATCH, OPTIONS', // Specify allowed methods
      'Access-Control-Allow-Headers': 'X-Requested-With, content-type, Authorization' // Specify allowed headers
    },
  },
};
