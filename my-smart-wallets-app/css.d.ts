// TypeScript 6 checks side-effect imports such as `import "./globals.css"`.
// Next 15's bundled types declare only CSS modules, so declare plain CSS here.
declare module "*.css";
