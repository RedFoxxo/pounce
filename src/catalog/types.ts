export interface CatalogField {
  name: string
  /** Targetprocess type name, e.g. `String`, `Int32`, `UserStory`. */
  type: string
  canSet: boolean
  canGet: boolean
  required: boolean
  deprecated: boolean
  description: string
}

export interface CatalogCollection extends CatalogField {
  canAdd: boolean
  canRemove: boolean
}

export interface CatalogResource {
  /** Singular name, e.g. `UserStory`. */
  name: string
  /** Plural path below `/api/v1`, e.g. `UserStories`. */
  path: string
  description: string
  /** Listed in `/api/v1/Index/meta` (false for probed history/conversion resources). */
  listed: boolean
  /** False when the index lists the resource but its `/meta` fails (e.g. SickLeave). */
  available: boolean
  canCreate: boolean
  canUpdate: boolean
  canDelete: boolean
  /** Base resources, e.g. `Assignable` for `UserStory`. */
  bases: string[]
  values: CatalogField[]
  references: CatalogField[]
  collections: CatalogCollection[]
}

export interface CatalogData {
  /** Instance the catalog was read from. */
  instance: string
  /** Targetprocess version reported by `Context`, when readable. */
  version?: string
  capturedAt: string
  resources: CatalogResource[]
}
