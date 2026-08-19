import { useEffect, useState } from "react";

import {
  activeResultId,
  scrollActiveResultIntoView,
} from "../features/quick-insert/model/navigation";

export function useActiveResultNavigation({
  resultKeys,
  resetToken,
  popupId,
  visible,
}: {
  resultKeys: readonly string[];
  resetToken: string;
  popupId: string;
  popupRole: "grid" | "listbox";
  visible: boolean;
}) {
  const [activeKey, setActiveKey] = useState<string | null>(
    resultKeys[0] ?? null,
  );
  const resolvedKey =
    resultKeys.length === 0
      ? null
      : activeKey && resultKeys.includes(activeKey)
        ? activeKey
        : resultKeys[0];

  useEffect(() => {
    setActiveKey(resultKeys[0] ?? null);
  }, [resetToken, resultKeys.length, resultKeys[0]]);

  const activeIndex =
    resolvedKey === null ? -1 : resultKeys.indexOf(resolvedKey);
  const getResultId = (key: string) => activeResultId(popupId, key);
  const activeDescendantId =
    visible && resolvedKey ? getResultId(resolvedKey) : undefined;

  useEffect(() => {
    if (activeDescendantId)
      scrollActiveResultIntoView(document.getElementById(activeDescendantId));
  }, [activeDescendantId]);

  const activateIndex = (index: number) => {
    const key = resultKeys[index];
    if (key !== undefined) setActiveKey(key);
  };
  const move = (direction: -1 | 1) => {
    if (resultKeys.length === 0) return;
    const next = Math.max(
      0,
      Math.min(
        (activeIndex < 0 ? 0 : activeIndex) + direction,
        resultKeys.length - 1,
      ),
    );
    activateIndex(next);
  };

  return {
    activeIndex,
    activeDescendantId,
    activateIndex,
    move,
    getResultId,
    comboboxProps: {
      role: "combobox" as const,
      "aria-controls": popupId,
      "aria-expanded": visible,
      "aria-autocomplete": "list" as const,
      "aria-activedescendant": activeDescendantId,
    },
  };
}
