export type DataClassification =
  "public" | "internal" | "confidential" | "restricted" | "unclassified";

const CLASSIFICATION_LABEL: Record<DataClassification, string> = {
  public: "Public",
  internal: "Internal",
  confidential: "Confidential",
  restricted: "Restricted",
  unclassified: "Unclassified",
};

export interface ClassificationBadgeProps {
  readonly classification: DataClassification;
}

/**
 * Design system "Status and classification badges" pattern: "Classification
 * badges show Public, Internal, Confidential, Restricted, or Unclassified
 * in full" -- always the full word, never an abbreviation or color alone.
 * Consumes the classification token triplets from
 * design-system/tokens.css.
 */
export function ClassificationBadge({ classification }: ClassificationBadgeProps) {
  return (
    <span className="pmc-classification-badge" data-classification={classification}>
      {CLASSIFICATION_LABEL[classification]}
    </span>
  );
}
