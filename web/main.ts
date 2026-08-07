// Vite serves this file from its own root, so the browser asks for it under the
// configured base. A `../src/…` entry escapes the root and loses that prefix,
// which leaves the dev server serving a page whose only script 404s.
import "../src/web-main";
