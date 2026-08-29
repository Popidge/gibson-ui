# GIBSON website

This directory contains the dependency-free GIBSON product site.

## Local preview

```bash
python3 -m http.server 4173 --directory website
```

Open <http://localhost:4173>.

## Vercel deployment

Import `Popidge/gibson-ui` in Vercel and use these project settings:

- Framework preset: `Other`
- Root directory: `website`
- Build command: leave empty
- Output directory: leave empty

Vercel will deploy the static files directly and create preview deployments for pull requests. Production deploys follow pushes to `main`.

Enable **Skip deployments when there are no changes to the root directory** in the Root Directory settings if product-only commits should not redeploy the site.

The website does not affect the GIBSON installer. `install.sh` downloads the checksum-verified binary archive from GitHub Releases, not the repository contents.
