import type { ReactElement } from "react";

import {
  QuickInsertResults,
  type SharedEntryResultsProps,
} from "../quick-insert/components/QuickInsertResults";

export function HistoryResults(props: SharedEntryResultsProps): ReactElement {
  return <QuickInsertResults {...props} view="history" />;
}
