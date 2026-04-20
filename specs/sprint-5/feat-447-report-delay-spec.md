# feat-447: 48-Hour Report Publication Delay for Grunnaðild (Reader) Tier

## Overview

The 48-hour delay for Reader subscribers is **already implemented** in `publishedFilter.ts`
and used across report endpoints. This spec covers: verifying correctness, finding and fixing
any endpoints that bypass the filter, and adding test coverage for the investor bypass path.

## What Already Exists

`apps/api/src/Utils/publishedFilter.ts`:

```ts
export function isReaderOnly(
  userAccess: { isAdmin: boolean; isAnalyst: boolean; isInvestor: boolean } | undefined,
): boolean {
  if (!userAccess) return true;
  return !userAccess.isAdmin && !userAccess.isAnalyst && !userAccess.isInvestor;
}
```

When `isReaderOnly()` returns `true`, report endpoints apply:

```ts
where.published = true;
where.publishDate = { [Op.lte]: new Date(Date.now() - 48 * 60 * 60 * 1000) };
```

Investors (`isInvestor: true`) return `false` from `isReaderOnly()` → no delay → immediate
access. This is correct by design.

## Audit Required

Before writing any code, audit every report endpoint. **Do not assume any endpoint is
already correct** — even the ones listed as "confirmed" below must be checked against the
actual source. An independent review found that several "confirmed" endpoints (`/get-single-report`,
`/get-report-pdf`, `/get-report-tables-by-reportid`, `/get-report-tables-by-forecastid`) only
check `published = true` and do NOT apply the 48h `publishDate` filter.

For every endpoint below: read the route handler and any service/data layer it calls. Confirm
that **both** `published = true` AND `publishDate <= NOW() - 48h` are applied when
`isReaderOnly()` is true. If either condition is missing, add it.

Endpoints to verify and fix if needed:

- `POST /all-reports`
- `POST /all-company-reports`
- `POST /get-single-report`
- `POST /get-report-pdf`
- `POST /get-report-tables-by-reportid`
- `POST /get-report-tables-by-forecastid`

The following also need verification:

- `POST /latest-summary` — unclear if delay is applied
- `GET /published-report-companies` — uses raw SQL `published = true`, no delay check
- `POST /get-peergroup-data-by-report` — data layer, unclear

For each unverified endpoint: read the route handler and service layer. If it returns
report content to readers without the 48h filter, add `isReaderOnly()` check.

## Correct Behaviour

| Role | Published > 48h | Published < 48h | Unpublished |
|------|----------------|-----------------|-------------|
| vs-readers | ✅ visible | ❌ hidden | ❌ hidden |
| vs-investors | ✅ visible | ✅ visible | ❌ hidden |
| vs-analysts | ✅ visible | ✅ visible | ✅ visible (own drafts) |
| vs-admins | ✅ visible | ✅ visible | ✅ visible (all) |

Reports are **absent** from reader results during the embargo — not shown as locked or coming soon.

## `publishedFilter.ts` Gap: `isInvestor` dependency on #446

`isReaderOnly()` checks `!isInvestor`. For this to work correctly, `req.userAccess.isInvestor`
must be set to `true` for `vs-investors` users by `roleMiddleware.ts`.

**Verify this is the case before any other work.** Read `roleMiddleware.ts` and confirm
`isInvestor` is populated from the `vs-investors` Cognito group. If not, fix it first.

## `published-report-companies` Gap

`GET /published-report-companies` uses raw SQL: `published = true AND isDeleted = false`.
This does not apply the 48h delay. FR-1 requires readers cannot see reports published
within the last 48h on **any** endpoint — including company list endpoints. This is not
deferrable.

**Fix:** Add the 48h filter to the raw SQL query when `isReaderOnly()` is true:

```ts
if (isReaderOnly(req.userAccess)) {
  // add: AND publishDate <= NOW() - INTERVAL '48 hours' to the WHERE clause
}
```

## Pre-check + Second Fetch Pattern

Several endpoints do a cheap access pre-check (`published = true`) on one query, then
run a second broader query to get the actual response. The checked row and the returned
row are not guaranteed to be the same record. If the 48h filter is only applied to the
pre-check query, the second fetch can return embargoed data.

**Fix:** The 48h filter must be applied to the **data-returning query**, not only the
pre-check. Affected endpoints (verify exact implementation by reading the source):
- `getSingleReport` — pre-checks reportId[0] then fetches report[0] separately
- `getPeerGroupDataByReportId` — similar pattern
- `getReportPdf` — similar pattern
- Table endpoints in `reportTables.ts`

For each: add `publishDate <= NOW() - 48h` to the WHERE clause in the actual data query,
not just to an access guard before it.

## Requirements

### Functional

- **FR-1**: `vs-readers` cannot see reports published within the last 48 hours on any endpoint.
- **FR-2**: `vs-investors` see all published reports immediately.
- **FR-3**: Reports are absent from reader results during embargo — no locked/coming-soon state.
- **FR-4**: `vs-analysts` and `vs-admins` see all reports (no filter change).
- **FR-5**: `isInvestor` correctly set from `vs-investors` Cognito group in `roleMiddleware.ts`.

### Non-Functional

- **NFR-1**: No new lint suppressions.
- **NFR-2**: All filter logic stays in service/data layer via `isReaderOnly()` — not duplicated in routes.
- **NFR-3**: `turbo run build lint typecheck test` passes.

## Testing

- [ ] `vs-readers` token: report published 47h ago not returned from `/all-reports`
- [ ] `vs-readers` token: report published 49h ago returned from `/all-reports`
- [ ] `vs-investors` token: report published 1h ago returned from `/all-reports`
- [ ] `vs-investors` token: report published 1h ago PDF downloadable via `/get-report-pdf`
- [ ] `vs-investors` token: report published 1h ago tables returned via `/get-report-tables-by-reportid`
- [ ] `vs-readers` token: `/get-single-report` for <48h report returns 404 or empty
- [ ] `isReaderOnly()` unit test: returns `true` for reader, `false` for investor, `false` for analyst
- [ ] Unpublished report not returned for `vs-readers` or `vs-investors`
- [ ] Unpublished report IS returned for `vs-analysts` (own drafts) and `vs-admins` (all)
- [ ] `vs-readers` token: `/latest-summary` does not include reports published <48h ago
- [ ] `vs-readers` token: `/published-report-companies` does not include companies whose only recent report is <48h old
- [ ] `vs-readers` token: `/get-peergroup-data-by-report` does not leak data for <48h reports

## Out of Scope

- Frontend "coming soon" or embargo countdown UI
- Email notification to readers when embargo lifts
- Admin override to grant reader early access

## Dependencies

- #446 (RBAC) should land first — `isInvestor` flag must be correctly populated before
  testing investor bypass. If implementing in parallel, verify `roleMiddleware.ts` manually.
