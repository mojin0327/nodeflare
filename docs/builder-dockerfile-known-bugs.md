# Builder Dockerfile Generation - Known Bugs

## Medium

### A. pyproject.toml + requirements.txt 共存時にrequirements.txtが無視される
- **場所**: `flyctl.rs` `generate_python_install_deps()`
- **条件**: `has_pyproject == true` かつ `has_requirements_txt == true`
- **現象**: pyprojectが優先されrequirements.txtが完全無視。実際の依存がrequirements.txtにある古いプロジェクトでビルド失敗

### B. Poetryプロジェクトの非検出
- **場所**: `flyctl.rs` `generate_python_install_deps()`
- **条件**: `pyproject.toml`に`[tool.poetry]`あり、`uv.lock`なし
- **現象**: `pip install .`が実行されるがpoetry-coreなしで失敗する可能性

### C. STDIO + 既存Dockerfile + build_commandが無視される
- **場所**: `flyctl.rs` `generate_stdio_dockerfile_with_existing()`
- **条件**: プロジェクトが独自Dockerfileを持ち、transportがstdio、かつbuild_commandが設定されている
- **現象**: `generate_stdio_dockerfile_with_existing`にbuild_commandが渡されないため完全無視

## Low

### D. Node.jsでnpm installが2重実行される
- **場所**: `flyctl.rs` `node_setup_section()` + `node_build_step()`
- **条件**: Node runtime、`build_command: "npm install"` または `"npm ci"` が設定されている
- **現象**: `node_setup_section`で自動実行 + build_commandでも実行で2重。失敗はしないがビルド時間増加
