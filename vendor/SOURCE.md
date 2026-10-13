# JevRouter SDK

`jevrouter-0.1.0.tgz` is the npm package built from
<https://github.com/BillionsBobby/JevRouter> at commit
`b1cdf80593ee530cedd99728a938f4e74ddd556b`.

The upstream MIT license is included as `package/LICENSE` in the archive.
The archive contains upstream compiled JavaScript, declarations and public
documentation. No XenoNet changes have been made to its implementation.

SHA-1 from `npm pack`: `008c41b2234874be560b0be37fa4ca8ae50d691a`.
The SHA-512 integrity in package-lock.json verifies the package at install.

To rebuild it, check out the exact upstream commit, install its development
dependencies, run `npm run build`, then `npm pack --ignore-scripts`. Compare
the output with the saved archive and review any difference before replacing
it. XenoNet uses this small local archive because production installs skip
prepare scripts and the upstream package has not been published on npm.
