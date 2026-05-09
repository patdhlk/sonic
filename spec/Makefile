# Build the sonic-executor specification with uv-managed Sphinx.

.PHONY: help install html clean strict

help:
	@echo "make install   Sync the uv venv from pyproject.toml"
	@echo "make html      Build HTML output into _build/html/"
	@echo "make strict    Build HTML with -W (warnings = errors); used by CI"
	@echo "make clean     Remove _build/"

install:
	uv sync

html:
	uv run sphinx-build -b html . _build/html

strict:
	uv run sphinx-build -W -b html . _build/html

clean:
	rm -rf _build
