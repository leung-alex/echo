import type { ReactElement } from "react";

import {
  QuickInsertResults,
  type SharedEntryResultsProps,
} from "../quick-insert/components/QuickInsertResults";

export function SavedItemsResults(
  props: SharedEntryResultsProps,
): ReactElement {
  return <QuickInsertResults {...props} view="favorites" />;
}
