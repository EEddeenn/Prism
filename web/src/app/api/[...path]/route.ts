/* Same-origin proxy to prism-serve.
 *
 * The browser fetches `/api/...` from this Next.js app; this handler forwards the
 * request server-side to the Rust prism-serve JSON API (PRISM_API_URL), so the
 * browser never deals with CORS and the upstream origin stays server-side. Only
 * GET is proxied — prism-serve is strictly read-only. */

import { NextRequest } from 'next/server';

const UPSTREAM = process.env.PRISM_API_URL ?? 'http://127.0.0.1:7831';

export const dynamic = 'force-dynamic';

export async function GET(
  req: NextRequest,
  { params }: { params: Promise<{ path: string[] }> },
) {
  const { path } = await params;
  const search = req.nextUrl.search;
  const target = `${UPSTREAM}/api/${path.map(encodeURIComponent).join('/')}${search}`;

  let upstream: Response;
  try {
    upstream = await fetch(target, { cache: 'no-store' });
  } catch {
    return Response.json(
      { error: 'prism-serve is unreachable (set PRISM_API_URL or start prism-serve)' },
      { status: 502 },
    );
  }

  const body = await upstream.text();
  return new Response(body, {
    status: upstream.status,
    headers: {
      'content-type': upstream.headers.get('content-type') ?? 'application/json; charset=utf-8',
      'cache-control': 'no-store',
    },
  });
}
