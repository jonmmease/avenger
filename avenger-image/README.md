# avenger-image

Images for Avenger scene graphs: the `RgbaImage` type, decoding, and the
resolvers through which renderers load the images that scenes name by URL.

## Images

`RgbaImage` holds an image's width, height, and straight-alpha RGBA pixels.
`RgbaImage::from_str` reads a `data:` URL of a PNG or an SVG document, or
fetches an `http` or `https` URL. `RgbaImage::decode` decodes fetched bytes: an
SVG document when the URL's path ends in `.svg`, which rasterizes at twice its
size, or else a format that the `image` crate reads.

## Resolvers

A scene can name an image by URL instead of holding its pixels. A renderer
then asks an `ImageResolver` for the image each time it draws. A resolver
answers each URL with an `ImageState`: `Ready` with the pixels, `Pending`, or
`Failed` with a reason, and requests an image that it holds no answer for.

- `image(url)` answers, and requests the image when the resolver has no answer.
- `prefetch(url)` requests the image behind every `image` request.
- `ready(url)` answers only from what the resolver holds, and never requests.
- `retain(urls)` returns an `ImageLease`, which keeps the images in memory
  until it drops.

A `HashMap` of URLs to images is a resolver that answers at once, for images
that an application already holds, such as test fixtures. A URL that it
doesn't hold fails.

## The image cache

`ImageCache` is a resolver that fetches images in the background and keeps
them in memory. `ImageCacheOptions` set its budget, its fetches, its fetcher,
and its disk cache.

- **Fetching.** The cache fetches on a bounded pool: up to 16 threads on
  native targets, which start when the cache has requests and exit when it
  has none, and up to 6 fetches with the browser's `fetch` on the web. Images
  that `image` requests start before images that `prefetch` requests, and
  prefetches leave two fetches free for them. Each image fetches once while
  the cache holds it. In the browser,
  `createImageBitmap` decodes the images off the main thread. A 2D canvas
  stores pixels premultiplied, so a translucent pixel can round differently
  than a native decode does.
- **Memory.** Decoded images stay within a byte budget, 128 MiB by default,
  and the least recently used go first. Leased images don't count toward the
  budget, and stay until their last lease drops. When the last lease that
  holds an image drops, a fetch for it that hasn't started is dropped, so a
  view that moves on doesn't wait for images that it no longer shows. A
  request without a lease stays queued until it runs.
- **Failures.** A failed image fails until a wait passes: 1 second after its
  first failure, doubling with each failure in a row, to 5 minutes. A 4xx
  response, or bytes that don't decode, waits 5 minutes at once. The next
  request after the wait fetches it again.
- **Changes.** `set_on_change` registers a callback that runs, outside the
  cache's lock, when an image that `image` asked for arrives, changes, or
  fails, and when a failed one can fetch again. Images that only `prefetch`
  asked for arrive without it, since no view draws them yet, and a failed
  prefetch that `image` then asks for runs it when its wait passes. avenger-app's
  `AvengerApp::with_images` registers one that wakes the app.
- **Waiting.** On native targets, `wait(urls, timeout)` requests images and
  waits until each is ready or failed, so an export or a test can draw them.

## Fetchers

An `ImageFetcher` sends a request and returns a `FetchResponse` with the
status, the caching headers, the validators, and the encoded bytes. It fails
only when no response arrives. A fetch takes the validators of a conditional
request.

`ReqwestImageFetcher`, with the `reqwest` feature on native targets, fetches
with one blocking reqwest client over rustls. It negotiates HTTP/2 where the
server offers it, so one connection carries concurrent fetches. It sends the
User-Agent that it's built with, and gives up after its timeout.
`default_image_fetcher` is one shared fetcher with avenger's User-Agent,
`avenger/<version> (+https://github.com/jonmmease/avenger)`, and a 30-second
timeout, which the cache and `RgbaImage::from_str` use unless they name
another.

## The disk cache

On native targets, `ImageCacheOptions::disk` keeps the fetched responses in a
directory, as their HTTP caching headers allow, so a later run draws them
without fetching them again.

- Each response is a file named from its URL: the directory, the scheme, the
  host, and the path. A URL with a query, or with characters outside
  letters, digits, `.`, `-`, and `_`, gets a hash of the URL in its name. A
  JSON file beside each response holds its URL, its validators, its
  `Cache-Control` header, when it goes stale, and when it was last used.
- Every file is written under a temporary name and renamed, so a partial
  write is never read. When the cache first reads the directory, it removes
  the temporary files and the responses without metadata that are older than
  an hour.
- A response stays fresh for its `max-age`, less its `Age`, or until its
  `Expires` date, or for 7 days without either. `no-cache` makes it stale at
  once, and `no-store` keeps it off the disk.
- A fresh response draws without a request. A stale one draws at once while
  its conditional request runs as a prefetch, and a 304 response refreshes its
  metadata. A stale response marked `must-revalidate` or `no-cache` waits for
  its conditional request instead.
- A byte budget, 50 MB by default, removes the least recently used responses.
- The pool's threads read and write the files, so a request never waits on
  the disk.

## Tile servers

A tile server sets its own terms. OpenStreetMap's
[tile usage policy](https://operations.osmfoundation.org/policies/tiles/)
asks each application to identify itself with a User-Agent, to cache tiles for
at least 7 days unless the headers say otherwise, to honor the caching
headers, to keep its fetches modest, and to credit OpenStreetMap. The cache's
pool bounds the fetches, and it drops the queued fetches of tiles that leave
the view. An application identifies itself by building a
`ReqwestImageFetcher` with its own User-Agent, keeps a disk cache, and
credits the tiles on its map, for example with avenger-guides' `Attribution`.

## Feature flags

- `svg` rasterizes SVG documents with resvg.
- `reqwest` fetches over HTTP on native targets with reqwest.
