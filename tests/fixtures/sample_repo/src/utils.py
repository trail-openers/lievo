import os
import sys

def process_data(data):
    result = []
    for item in data:
        if item > 0:
            result.append(item * 2)
    return result

def load_config(path):
    try:
        with open(path) as f:
            return f.read()
    except Exception as e:
        return None
