import click
from pathlib import Path
import toml
import json

root = Path(__file__).parent.parent.absolute()


@click.command()
@click.argument('version')
def bump_version(version):
    print(f"Updating version to {version}")

    # Handle Cargo.toml files
    cargo_packages = [
        "avenger",
        "avenger-vega",
        "avenger-vega-renderer",
        "avenger-wgpu",
        "avenger-vega-test-data",
        "avenger-python",
    ]

    # Handle package.json files
    package_jsons = [
        "avenger-vega-renderer"
    ]

    # Handle pyproject.toml files
    pyproject_tomls = [
        "avenger-python"
    ]

    for package in cargo_packages:
        cargo_toml_path = (root / package / "Cargo.toml")
        cargo_toml = toml.loads(cargo_toml_path.read_text())
        # Update this package's version
        cargo_toml["package"]["version"] = version

        # Look for local workspace dependencies and update their versions
        for dep_type in ["dependencies", "dev-dependencies", "build-dependencies"]:
            for p, props in cargo_toml.get(dep_type, {}).items():
                if isinstance(props, dict) and props.get("path", "").startswith("../avenger"):
                    props["version"] = version

        # Fix quotes in target so that they don't get double escaped when written back out
        new_target = {}
        for target, val in cargo_toml.get("target", {}).items():
            unescaped_target = target.replace(r'\"', '"')
            new_target[unescaped_target] = val
        if new_target:
            cargo_toml["target"] = new_target

        cargo_toml_path.write_text(toml.dumps(cargo_toml))
        print(f"Updated version in {cargo_toml_path}")

    for package_json_dir in package_jsons:
        for fname in ["package.json", "package-lock.json"]:
            package_json_path = root / package_json_dir / fname
            package_json = json.loads(package_json_path.read_text())
            package_json["version"] = version
            package_json_path.write_text(json.dumps(package_json, indent=2))
            print(f"Updated version in {package_json_path}")

    # pyproject.toml
    for toml_dir in pyproject_tomls:
        pyproject_toml_path = (root / toml_dir / "pyproject.toml")
        pyproject_toml = toml.loads(pyproject_toml_path.read_text())
        pyproject_toml["project"]["version"] = version
        pyproject_toml_path.write_text(toml.dumps(pyproject_toml))
        print(f"Updated version in {pyproject_toml_path}")


if __name__ == '__main__':
    bump_version()
