#!/usr/bin/env python3

import matplotlib.pyplot as plt
from matplotlib.widgets import Button
from matplotlib.axes import Axes
# from datetime import datetime
import pandas as pd
import seaborn as sns
import argparse
import numpy as np
import linecache
import json

# Default Arguments
DEFAULT_DATASET_PATH = './.logs/run_2024-06-24_20-55.jsonl'
DEFAULT_EXPORT_PATH = '.export'
DEFAULT_PLOT_FILTERED = True
DEFAULT_EXPORT_RECORDING_INDEX = 17
DEFAULT_RESET_TIMESTAMPS = True
DEFAULT_FILTER_KEEP_REST = False
DEFAULT_EXPORT_PATH = './.export/'
DEFAULT_DIASHOW_KBPS = False

# RNTI Filterting
DCI_THRESHOLD = 0
EMPTY_DCI_RATIO_THRESHOLD = 0.99
SAMPLES_THRESHOLD = 5

MAX_TOTAL_UL_FACTOR = 200.0
MIN_TOTAL_UL_FACTOR = 0.005 # x% of the expected UL traffic
MAX_UL_PER_DCI_THRESHOLD = 5_000_000
MIN_OCCURENCES_FACTOR = 0.05

# Plotting
PLOT_SCATTER_MARKER_SIZE = 10
MARKERS = ['o', 's', '^', 'd', 'p', 'h', '*', '>', 'v', '<', 'x']

sns.set(style="darkgrid")

##########
# Helpers
##########

def print_debug(msg: str):
    print(msg)


def print_info(msg: str):
    print(msg)


def count_lines(file_path: str) -> int:
    with open(file_path, 'r') as file:
        return sum(1 for _ in file)


def read_single_dataset(file_path: str, line_number: int) -> dict:
    try:
        return json.loads(linecache.getline(file_path, line_number).strip())
    except Exception as e:
        raise Exception(f"An error occured reading dataset at {file_path}:{line_number}\n{e}")


def save_plot(file_path):
    plt.tight_layout()
    plt.savefig(file_path)
    print_debug(f"Saved file: {file_path}")


##########
# Diashow
##########

def diashow(settings):
    data = [None for _ in range(count_lines(settings.path))]
    print(f"Number of datasets in the file: {len(data)}")

    _, ax = plt.subplots()
    tracker = IndexTracker(ax, data, settings)

    axprev = plt.axes([0.7, 0.01, 0.1, 0.075])
    axnext = plt.axes([0.81, 0.01, 0.1, 0.075])
    bnext = Button(axnext, 'Next')
    bnext.on_clicked(tracker.next)
    bprev = Button(axprev, 'Previous')
    bprev.on_clicked(tracker.prev)

    plt.show()


class FilteredRecording:
    def __init__(self):
        self.df: pd.DataFrame = pd.DataFrame()
        self.rtt_mean: int = 0
        self.rtt_median: int = 0
        self.cwnd_mean: int = 0
        self.cwnd_median: int = 0


def filter_dataset(settings, raw_dataset) -> FilteredRecording:

    result: FilteredRecording = FilteredRecording()
    result.rtt_mean = raw_dataset['rtt_mean']
    result.rtt_median = raw_dataset['rtt_median']
    result.cwnd_mean = raw_dataset['cwnd_mean']
    result.cwnd_median = raw_dataset['cwnd_median']

    timedata =  raw_dataset['timedata']
    # converted_timestamp = np.datetime64(int(timestamp), 'us')
    df = pd.DataFrame.from_dict(timedata, orient='index')
    df.index = pd.to_datetime(df.index, unit='us')

    result.df = df

    return result


class IndexTracker:
    def __init__(self, ax, data: list, settings):
        self.ax = ax
        self.data = data # : list of FilteredRecording
        self.settings = settings
        self.index = 0
        if self.check_data(self.index):
            self.plot()

    def plot(self):
        fig = self.ax.figure

        self.ax.clear()
        for ax in fig.axes:
            if ax is not self.ax and ax.get_ylabel() != '':
                fig.delaxes(ax)

        df = self.data[self.index].df
        # TODO: Evaluate units
        # if self.settings.kbps:
        #     df = df.resample('1s').sum().mul(8).div(1000)

        plot_df(DEFAULT_DIASHOW_PLOT_TYPE_CHOICES[self.settings.plot_type], df, axes=self.ax)

    def check_data(self, file_index) -> bool:
        if isinstance(self.data[file_index], FilteredRecording):
            return True
        elif self.data[self.index] == None:
            # Read dataset
            try:
                raw_data = read_single_dataset(self.settings.path, self.index + 1)
                filtered_data = filter_dataset(self.settings, raw_data)
                if filtered_data is not None:
                    self.data[file_index] = filtered_data
                    print(f"Successfully loaded dataset {self.settings.path}:{file_index}")
                    return True
            except Exception as e:
                print(f"Dataset not plottable: {e}")

        return False

    def next(self, _):
        if count_lines(self.settings.path) != len(self.data):
            self.data = [None for _ in range(count_lines(self.settings.path))]
        self.index = (self.index + 1) % len(self.data)
        if self.check_data(self.index):
            self.plot()

    def prev(self, _):
        if count_lines(self.settings.path) != len(self.data):
            self.data = [None for _ in range(count_lines(self.settings.path))]
        self.index = (self.index - 1) % len(self.data)
        if self.check_data(self.index):
            self.plot()


def plot_df(func, df: pd.DataFrame, axes=None, legend=True):
    ax: Axes = axes

    if ax is None:
        _, ax = plt.subplots()

    func(ax, df)

    # ax.set_title('Scatter Plot of UL Bytes over Time')
    ax.tick_params(axis='x', rotation=45)
    ax.set_xlabel('Timestamp (seconds)', fontsize=28)
    ax.set_ylabel('RTT (us)', fontsize=28)
    ax.tick_params(axis='x', labelsize=24)
    ax.tick_params(axis='y', labelsize=24)

    if legend:
        ax.legend(fontsize=18)

    if ax is None:
        plt.show()
    else:
        plt.draw()


def plot_pandas_scatter(ax, df: pd.DataFrame):
    for i, column in enumerate(df.columns):
        ax.scatter(df.index, df[column], label=column, marker=MARKERS[i % len(MARKERS)], s=PLOT_SCATTER_MARKER_SIZE)

def plot_pandas_scatter_twinx(ax, df: pd.DataFrame):
    ax.scatter(df.index, df[df.columns[0]], label=df.columns[0], marker=MARKERS[0 % len(MARKERS)], s=PLOT_SCATTER_MARKER_SIZE)
    ax.set_ylabel(df.columns[0])
    
    ax2 = ax.twinx()
    
    ax2.scatter(df.index, df[df.columns[1]], label=df.columns[1], marker=MARKERS[1 % len(MARKERS)], s=PLOT_SCATTER_MARKER_SIZE, color='r')
    ax2.set_ylabel(df.columns[1])


def plot_pandas_line(ax, df):
    for i, column in enumerate(df.columns):
        x_values = df.index  # Convert index to NumPy array
        y_values = df[column].values
        ax.plot(x_values, y_values, marker=MARKERS[i % len(MARKERS)], label=column)


def plot_pandas_line_twinx(ax, df):
    ax.plot(df.index.to_numpy(), df[df.columns[0]].values, label=df.columns[0], marker=MARKERS[0 % len(MARKERS)])
    ax.set_ylabel(df.columns[0])
    
    ax2 = ax.twinx()
    
    ax2.plot(df.index.to_numpy(), df[df.columns[1]].values, label=df.columns[1], marker=MARKERS[1 % len(MARKERS)], color='r')
    ax2.set_ylabel(df.columns[1])


def log_bins(data):
    return np.logspace(np.log10(20), np.log10(data.max()), num=50)


def plot_pandas_hist_log(ax, df, bin_func=log_bins):
    all_ul_bytes = df.stack().values
    bins = bin_func(all_ul_bytes)

    # Create logarithmic bins
    for column in df.columns:
        ax.hist(df[column], bins=bins, edgecolor='k', alpha=0.7, label=column)

    ax.set_xscale('symlog')
    # ax.set_title('Histogram of UL Bytes')


def plot_pandas_hist(ax, df):

    df.plot(kind='hist', bins=50, alpha=0.5, ax=ax)
    ax.set_xlabel('UL Bytes')
    ax.set_ylabel('Frequency')
    # ax.set_title('Histogram of UL Bytes')

DEFAULT_DIASHOW_PLOT_TYPE = 'scatter-twinx'
DEFAULT_DIASHOW_PLOT_TYPE_CHOICES = {
    'scatter': plot_pandas_scatter,
    'scatter-twinx': plot_pandas_scatter_twinx,
    'line': plot_pandas_line,
    'line-twinx': plot_pandas_line_twinx,
    'hist': plot_pandas_hist,
}

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description='Display UL traffic patterns from a dataset.')
    parser.add_argument('--path',
                        type=str,
                        default=DEFAULT_DATASET_PATH,
                        help=f'Path to the dataset file (default: {DEFAULT_DATASET_PATH})')
    parser.add_argument('--reset-timestamps',
                        type=bool,
                        default=DEFAULT_RESET_TIMESTAMPS,
                        help=f'Reset timestamps to 00:00 (default: {DEFAULT_RESET_TIMESTAMPS})')

    subparsers = parser.add_subparsers(dest='command', required=True)

    # diashow subcommand
    parser_diashow = subparsers.add_parser('diashow', help='Run diashow mode')
    parser_diashow.add_argument('--kbps',
                                type=bool,
                                default=DEFAULT_DIASHOW_KBPS,
                                help='Resample to kbps (default: {DEFAULT_DIASHOW_KBPS})')
    parser_diashow.add_argument('--plot-type',
                                type=str,
                                choices=list(DEFAULT_DIASHOW_PLOT_TYPE_CHOICES.keys()),
                                default=DEFAULT_DIASHOW_PLOT_TYPE,
                                help='The type of the plot (default: {DEFAULT_DIASHOW_PLOT_TYPE})')

    args = parser.parse_args()

    if args.command == 'diashow':
        diashow(args)
