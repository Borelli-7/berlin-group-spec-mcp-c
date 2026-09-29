-- Berlin Group specification catalog: authoritative metadata and provenance.

CREATE TABLE index_meta (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);

CREATE TABLE documents (
    source_id        TEXT PRIMARY KEY NOT NULL,
    document_id      TEXT NOT NULL,
    kind             TEXT NOT NULL,
    version          TEXT NOT NULL,
    authority        TEXT NOT NULL,
    precedence       INTEGER NOT NULL,
    title            TEXT NOT NULL,
    rel_path         TEXT NOT NULL,
    sha256           TEXT,
    size_bytes       INTEGER,
    status           TEXT NOT NULL,
    fingerprint      TEXT NOT NULL,
    extractor        TEXT,
    page_count       INTEGER,
    metadata_json    TEXT NOT NULL DEFAULT '{}',
    diagnostics_json TEXT NOT NULL DEFAULT '[]',
    indexed_at_unix  INTEGER NOT NULL,
    json             TEXT NOT NULL
);
CREATE INDEX documents_version ON documents(version, kind);

CREATE TABLE pages (
    source_id  TEXT NOT NULL REFERENCES documents(source_id) ON DELETE CASCADE,
    page       INTEGER NOT NULL,
    status     TEXT NOT NULL,
    text       TEXT NOT NULL,
    char_count INTEGER NOT NULL,
    sha256     TEXT NOT NULL,
    PRIMARY KEY (source_id, page)
);

CREATE TABLE chunks (
    chunk_id  TEXT PRIMARY KEY NOT NULL,
    source_id TEXT NOT NULL REFERENCES documents(source_id) ON DELETE CASCADE,
    version   TEXT NOT NULL,
    kind      TEXT NOT NULL,
    page      INTEGER,
    section   TEXT,
    ordinal   INTEGER NOT NULL,
    locator   TEXT NOT NULL,
    text      TEXT NOT NULL,
    sha256    TEXT NOT NULL,
    json      TEXT NOT NULL
);
CREATE INDEX chunks_source_page ON chunks(source_id, page, ordinal);
CREATE INDEX chunks_source_section ON chunks(source_id, section);

CREATE TABLE openapi_operations (
    id           INTEGER PRIMARY KEY,
    source_id    TEXT NOT NULL REFERENCES documents(source_id) ON DELETE CASCADE,
    version      TEXT NOT NULL,
    method       TEXT NOT NULL,
    path         TEXT NOT NULL,
    path_key     TEXT NOT NULL,
    operation_id TEXT,
    locator      TEXT NOT NULL,
    sha256       TEXT NOT NULL,
    json         TEXT NOT NULL,
    UNIQUE (source_id, method, path)
);
CREATE INDEX openapi_operations_lookup ON openapi_operations(version, method, path_key);
CREATE INDEX openapi_operations_path ON openapi_operations(source_id, path);

CREATE TABLE openapi_schemas (
    id        INTEGER PRIMARY KEY,
    source_id TEXT NOT NULL REFERENCES documents(source_id) ON DELETE CASCADE,
    version   TEXT NOT NULL,
    name      TEXT NOT NULL,
    locator   TEXT NOT NULL,
    sha256    TEXT NOT NULL,
    refs_json TEXT NOT NULL,
    json      TEXT NOT NULL,
    UNIQUE (source_id, name)
);
CREATE INDEX openapi_schemas_lookup ON openapi_schemas(version, name);

CREATE TABLE requirements (
    id      TEXT PRIMARY KEY NOT NULL,
    version TEXT NOT NULL,
    title   TEXT NOT NULL,
    sha256  TEXT NOT NULL,
    json    TEXT NOT NULL
);

-- source_id intentionally has no foreign key: dangling citations must remain visible.
CREATE TABLE requirement_sources (
    requirement_id TEXT NOT NULL REFERENCES requirements(id) ON DELETE CASCADE,
    ordinal        INTEGER NOT NULL,
    source_id      TEXT NOT NULL,
    locator        TEXT NOT NULL,
    sha256_pin     TEXT,
    PRIMARY KEY (requirement_id, ordinal)
);
CREATE INDEX requirement_sources_source ON requirement_sources(source_id);

CREATE TABLE requirement_endpoints (
    requirement_id TEXT NOT NULL REFERENCES requirements(id) ON DELETE CASCADE,
    method         TEXT NOT NULL,
    path           TEXT NOT NULL,
    PRIMARY KEY (requirement_id, method, path)
);
