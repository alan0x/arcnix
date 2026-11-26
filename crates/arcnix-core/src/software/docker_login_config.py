"""Print only registry and credential-helper names for the Docker logins dialog."""
import json
import sys


def metadata(config):
    auths = config.get("auths") or {}
    helpers = config.get("credHelpers") or {}
    store = config.get("credsStore")
    if not isinstance(auths, dict) or not isinstance(helpers, dict):
        raise ValueError()
    if not all(isinstance(helper, str) for helper in helpers.values()):
        raise ValueError()
    if store is not None and not isinstance(store, str):
        raise ValueError()
    return {"auths": dict.fromkeys(auths, {}), "credHelpers": helpers, "credsStore": store}


if __name__ == "__main__":
    try:
        with open(sys.argv[1], "rb") as source:
            data = source.read(65537)
        if len(data) > 65536:
            raise ValueError()
        print(json.dumps(metadata(json.loads(data))))
    except (OSError, ValueError, AttributeError, RecursionError):
        sys.exit("Docker's config.json isn't readable")
