
## copy-readwrite (MiB/s)

| threads | tokio | tokio-chunked | rayon | compio | tokio/compio |
|---|---|---|---|---|---|
| 1 | 1313 | 2818 | 2810 | 729 | 1.80 |
| 16 | 5880 | 10627 | 10661 | 2127 | 2.76 |

## dispatch (kops/s)

| threads | tokio | tokio-chunked | rayon | compio | tokio/compio |
|---|---|---|---|---|---|
| 1 | 14 | 172538 | 365358 | 3994 | 0.00 |
| 16 | 35 | 140041 | 27582 | 11921 | 0.00 |

## enumerate (entries/s)

| threads | tokio | tokio-chunked | rayon | compio | tokio/compio |
|---|---|---|---|---|---|
| 1 | 659986 | 675349 | 669301 | 664219 | 0.99 |
| 16 | 4877877 | 4205537 | 4059384 | 4239518 | 1.15 |

## perop (kops/s)

| threads | tokio | tokio-chunked | rayon | compio | tokio/compio |
|---|---|---|---|---|---|
| 1 | 14 | 2282 | 2148 | 784 | 0.02 |
| 16 | 35 | 12246 | 12287 | 1605 | 0.02 |

## randread-multi (MiB/s)

| threads | tokio | tokio-chunked | rayon | compio | tokio/compio |
|---|---|---|---|---|---|
| 1 | 54 | 4250 | 3944 | 1981 | 0.03 |
| 16 | 136 | 31519 | 33694 | 3190 | 0.04 |
| 64 | 148 | 29993 | 17270 | 3286 | 0.04 |

## randread (MiB/s)

| threads | tokio | tokio-chunked | rayon | compio | tokio/compio |
|---|---|---|---|---|---|
| 1 | 56 | 4136 | 3950 | 2016 | 0.03 |
| 16 | 131 | 29794 | 32358 | 3284 | 0.04 |
| 64 | 161 | 31370 | 27348 | 3191 | 0.05 |

## seqread (MiB/s)

| threads | tokio | tokio-chunked | rayon | compio | tokio/compio |
|---|---|---|---|---|---|
| 1 | 12345 | 18155 | 18112 | 17886 | 0.69 |
| 16 | 8060 | 7966 | 7852 | 7977 | 1.01 |

## smallfiles (MiB/s)

| threads | tokio | tokio-chunked | rayon | compio | tokio/compio |
|---|---|---|---|---|---|
| 1 | 98 | 1384 | 1389 | 1076 | 0.09 |
| 16 | 272 | 10422 | 12438 | 1644 | 0.17 |
