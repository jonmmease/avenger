# image-resources

Image marks that load by URL. The scene names four of
[vega-datasets](https://github.com/vega/vega-datasets)' sample images by URL,
with their size and a gray placeholder. The last URL doesn't exist.

Each image draws its placeholder until it arrives. The app's avenger-image
`ImageCache` fetches the images on its threads, and `AvengerApp::with_images`
wakes the window as each one arrives, which draws it without a new scene. The
image that fails to load keeps its placeholder.

```sh
cargo run --release -p image-resources
```
