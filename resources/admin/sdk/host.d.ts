/** An error from the panel's API (`props.api`): status, stable code, field errors. */
export declare class ApiError extends Error {
  readonly status: number;
  get code(): string;
  get fieldErrors(): Record<string, string[]>;
  get formErrors(): string[];
  /** The write may have been applied (the answer was lost). */
  get maybeApplied(): boolean;
}

/** The panel's versions. */
export declare const versions: Readonly<{ api: 2; react: string; reactQuery: string; ui: number }>;
