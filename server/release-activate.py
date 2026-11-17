"""Run the verified source's activation plan with the caller's server lock held."""
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import release

if __name__ == "__main__":
    release.activate(Path(__file__).resolve().parents[1], *sys.argv[1:])
