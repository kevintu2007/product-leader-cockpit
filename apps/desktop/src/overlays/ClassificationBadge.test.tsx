import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { ClassificationBadge, type DataClassification } from "./ClassificationBadge";

describe("ClassificationBadge", () => {
  it.each<[DataClassification, string]>([
    ["public", "Public"],
    ["internal", "Internal"],
    ["confidential", "Confidential"],
    ["restricted", "Restricted"],
    ["unclassified", "Unclassified"],
  ])("spells %s out in full as %s, never abbreviated", (classification, label) => {
    render(<ClassificationBadge classification={classification} />);
    expect(screen.getByText(label)).toBeVisible();
  });

  it("exposes the classification as a data attribute for non-color styling hooks", () => {
    render(<ClassificationBadge classification="restricted" />);
    expect(screen.getByText("Restricted")).toHaveAttribute("data-classification", "restricted");
  });
});
