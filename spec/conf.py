"""Sphinx configuration for the sonic-executor specification."""

import json
from pathlib import Path

# -- Project information -------------------------------------------------------

project = "sonic-executor — Specification"
author = "Patrick Dahlke"
copyright = "2026, Patrick Dahlke"
release = "0.1.0"

# -- General configuration -----------------------------------------------------

extensions = [
    "myst_parser",
    "sphinx_needs",
    "sphinx_hextra",
    "sphinxcontrib.mermaid",
]

templates_path = ["_templates"]
exclude_patterns = [
    "_build",
    "Thumbs.db",
    ".DS_Store",
    ".venv",
    "README.md",
    ".pharaoh",
    # `scripts/` hosts the Node.js mermaid validator (validate-mermaid.mjs)
    # and its npm dependency tree. None of it is spec content; MyST would
    # otherwise parse every README.md / .md file under scripts/node_modules
    # and emit thousands of xref-missing warnings. The validator is invoked
    # by CI directly via `npm run validate-mermaid`, not through Sphinx.
    "scripts",
]

# Allow .rst and .md side by side.
source_suffix = {
    ".rst": "restructuredtext",
    ".md": "markdown",
}

# Treat warnings as errors when invoked with -W (CI uses this).
nitpicky = False

# -- sphinx-needs configuration -----------------------------------------------

# Read need types and link types from ubproject.toml. Keeps directive/prefix/
# link-type declarations out of conf.py so tooling (pharaoh, ubc) can consume
# them as data without parsing Python.
needs_from_toml = "ubproject.toml"

# Schema validation (sphinx-needs schema docs:
# https://sphinx-needs.readthedocs.io/en/latest/schema/index.html). Rules live
# in spec/schemas.json so they're editable as data. `severity: violation` is
# the default and is caught by `sphinx-build -W` in CI. See spec/schemas.json
# for the rule set; current scope is ID format per type, status enum, and
# status=implemented network contracts.
needs_schema_validation_enabled = True
with (Path(__file__).parent / "schemas.json").open("r", encoding="utf-8") as _fh:
    needs_schema_definitions = json.load(_fh)

# -- HTML output (sphinx-hextra theme) -----------------------------------------

html_theme = "sphinx_hextra"
html_static_path = ["_static"]
html_title = project

# Canonical URL for the published site (GitHub Pages → patdhlk.com/sonic/).
# Affects only metadata (sitemaps, canonical links); does not change asset paths.
html_baseurl = "https://patdhlk.com/sonic/"
