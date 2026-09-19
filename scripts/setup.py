#!/usr/bin/env python3
"""Create a local bootstrap administrator without writing credentials to stdout."""
import os
from pathlib import Path
import secrets
root = Path(__file__).resolve().parent.parent
target = root / '.env'
if target.exists():
    print('.env already exists; kept existing configuration.')
else:
    config = (root / '.env.example').read_text().replace(
        'replace-with-a-unique-password-at-least-12-characters', secrets.token_urlsafe(32))
    fd = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, 'w') as stream:
        stream.write(config)
    print('Created .env with a random administrator password (file permissions 0600).')
    print('Administrator: admin@unskilled.local. Read ADMIN_PASSWORD in .env to sign in.')
