// SPDX-License-Identifier: AGPL-3.0-or-later
// Shared table classes so every list reads the same.
export const table = {
  wrap: 'relative w-full overflow-x-auto',
  table: 'w-full caption-bottom text-sm',
  head: '[&_tr]:border-b',
  headCell: 'h-10 px-3 text-left align-middle text-[13px] font-medium whitespace-nowrap text-muted-foreground first:pl-5 last:pr-5',
  body: '[&_tr:last-child]:border-0',
  row: 'border-b transition-colors hover:bg-muted/40',
  cell: 'px-3 py-2.5 align-middle first:pl-5 last:pr-5',
};
