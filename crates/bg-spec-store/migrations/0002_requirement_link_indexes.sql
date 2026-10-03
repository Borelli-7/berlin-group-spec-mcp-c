-- Indexed requirement lookups (by version, by endpoint method/path, by source locator kind).
CREATE INDEX IF NOT EXISTS requirements_version ON requirements(version);
CREATE INDEX IF NOT EXISTS requirement_endpoints_route ON requirement_endpoints(method, path);
