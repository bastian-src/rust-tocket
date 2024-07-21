#!/usr/bin/env python3

import argparse
import subprocess
import time


# Default Arguments
DEFAULT_PORT = 9393
DEFAULT_NOF_REQUESTS = 50
DEFAULT_REQUESTS_SPACING_MS = 1000
DEFAULT_PATH = '/pbe/init_and_upper'
DEFAULT_PROTO = 'http'
DEFAULT_PROTO_CHOICES = ['http', 'https']


def perform_request(proto, address, port, path):
    url = f"{proto}://{address}:{port}{path}"
    command = ['wget', '-O', '/dev/null', url]

    try:
        result = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, check=True)
        stdout = result.stdout
        print(f"{stdout}\n")
    except subprocess.CalledProcessError as e:
        print(f"An error occurred while making the wget request: {e}")

def main(args):
    for _ in range(args.nof_requests):
        perform_request(args.proto, args.address, args.port, args.path)
        time.sleep(args.request_spacing_ms / 1000)

def parse():
    parser = argparse.ArgumentParser(description='Perform multiple wget requests against a specific address.')

    parser.add_argument('address',
                        metavar='ADDRESS',
                        type=str,
                        help='Address to perform wget requests')
    parser.add_argument('--proto',
                        type=str,
                        default=DEFAULT_PROTO,
                        choices=DEFAULT_PROTO_CHOICES,
                        help=f'URL protocol (default: {DEFAULT_PROTO})')
    parser.add_argument('--path',
                        default=DEFAULT_PATH,
                        type=str,
                        help=f'URL path (default: {DEFAULT_PATH})')
    parser.add_argument('--port',
                        type=int,
                        default=DEFAULT_PORT,
                        help=f'Destination port (default: {DEFAULT_PORT})')
    parser.add_argument('--nof-requests',
                        type=int,
                        default=DEFAULT_NOF_REQUESTS,
                        help=f'Number of wget requests (default: {DEFAULT_NOF_REQUESTS})')
    parser.add_argument('--request-spacing-ms',
                        type=int,
                        default=DEFAULT_REQUESTS_SPACING_MS,
                        help=f'Waiting time between requests (default: {DEFAULT_REQUESTS_SPACING_MS})')
    return parser.parse_args()


if __name__ == "__main__":
    args = parse()
    main(args)
