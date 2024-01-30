import click
from pathlib import Path
import toml

root = Path(__file__).parent.parent.absolute()


@click.command()
@click.argument('version')
def bump_version(version):
    print(f"Updating version to {version}")

    # Handle Cargo.toml files
    cargo_packages = [
        "avenger",
        "avenger-vega",
        "avenger-wgpu",
        "avenger-vega-test-data",
        "avenger-python",
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


if __name__ == '__main__':
    bump_version()
