"""Sphinx configuration for the sonic-executor specification."""

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
]

templates_path = ["_templates"]
exclude_patterns = ["_build", "Thumbs.db", ".DS_Store", ".venv", "README.md", ".pharaoh"]

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

# -- HTML output (sphinx-hextra theme) -----------------------------------------

html_theme = "sphinx_hextra"
html_static_path = ["_static"]
html_title = project

# Canonical URL for the published site (GitHub Pages → patdhlk.com/sonic/).
# Affects only metadata (sitemaps, canonical links); does not change asset paths.
html_baseurl = "https://patdhlk.com/sonic/"
