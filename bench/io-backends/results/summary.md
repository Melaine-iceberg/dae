
## copy-kernel (MiB/s)

| threads | tokio | tokio-chunked | rayon | compio | tokio/compio |
|---|---|---|---|---|---|
| 1 | 1189 (0.8s cpu) | 1394 (0.7s cpu) | 1350 (0.7s cpu) | 1041 (0.9s cpu) | 1.14 |
| 16 | 1985 | 4266 | 4255 | 1984 | 1.00 |

## copy-readwrite (MiB/s)

| threads | tokio | tokio-chunked | rayon | compio | tokio/compio |
|---|---|---|---|---|---|
| 1 | 1282 (0.8s cpu) | 1320 (0.8s cpu) | 1256 (0.8s cpu) | 614 (1.1s cpu) | 2.09 |
| 16 | 1989 | 4232 | 3905 | 1449 | 1.37 |

## dispatch (kops/s)

| threads | tokio | tokio-chunked | rayon | compio | tokio/compio |
|---|---|---|---|---|---|
| 1 | 220 (0.8s cpu) | 156128 | 297708 | 12881 | 0.02 |
| 16 | 320 | 285878 | 634921 | 17676 | 0.02 |

## enumerate (entries/s)

| threads | tokio | tokio-chunked | rayon | compio | tokio/compio |
|---|---|---|---|---|---|
| 1 | 2911337 | 3096557 | 3240052 | 3288972 | 0.89 |
| 16 | 17390968 | 17795789 | 17236956 | 17416203 | 1.00 |

## perop (kops/s)

| threads | tokio | tokio-chunked | rayon | compio | tokio/compio |
|---|---|---|---|---|---|
| 1 | 127 (0.2s cpu) | 1491 | 1486 | 449 | 0.28 |
| 16 | 197 | 745 | 736 | 650 | 0.30 |

## randread-big-multi (MiB/s)

| threads | tokio | tokio-chunked | rayon | compio | tokio/compio |
|---|---|---|---|---|---|
| 1 | 339 (0.4s cpu) | 1123 | 1193 | 772 (0.2s cpu) | 0.44 |
| 16 | 755 | 1357 | 1399 | 734 | 1.03 |
| 64 | 515 | 1416 | 1508 | 760 | 0.68 |

## randread-big (MiB/s)

| threads | tokio | tokio-chunked | rayon | compio | tokio/compio |
|---|---|---|---|---|---|
| 1 | 25 (1.5s cpu) | 1022 (0.1s cpu) | 1036 | 630 (0.2s cpu) | 0.04 |
| 16 | 361 | 512 | 520 | 762 | 0.47 |
| 64 | 355 | 488 | 466 | 709 | 0.50 |

## randread-multi (MiB/s)

| threads | tokio | tokio-chunked | rayon | compio | tokio/compio |
|---|---|---|---|---|---|
| 1 | 352 (1.0s cpu) | 1234 (0.2s cpu) | 1200 (0.2s cpu) | 794 (0.3s cpu) | 0.44 |
| 16 | 670 | 1468 | 1445 | 936 | 0.72 |
| 64 | 828 | 1429 | 1511 | 953 | 0.87 |

## randread (MiB/s)

| threads | tokio | tokio-chunked | rayon | compio | tokio/compio |
|---|---|---|---|---|---|
| 1 | 339 (1.0s cpu) | 1238 (0.2s cpu) | 1240 (0.2s cpu) | 799 (0.3s cpu) | 0.42 |
| 16 | 539 | 787 | 783 | 961 | 0.56 |
| 64 | 523 | 773 | 754 | 968 | 0.54 |

## seqread-big (MiB/s)

| threads | tokio | tokio-chunked | rayon | compio | tokio/compio |
|---|---|---|---|---|---|
| 1 | 2936 (7.2s cpu) | 2779 (7.7s cpu) | 2806 (7.1s cpu) | 2002 (1.6s cpu) | 1.47 |
| 16 | 2756 (9.2s cpu) | 2055 (10.3s cpu) | 2185 (10.2s cpu) | 2845 (1.6s cpu) | 0.97 |

## seqread (MiB/s)

| threads | tokio | tokio-chunked | rayon | compio | tokio/compio |
|---|---|---|---|---|---|
| 1 | 5732 (1.4s cpu) | 7344 (1.1s cpu) | 7520 (1.1s cpu) | 7683 (1.1s cpu) | 0.75 |
| 16 | 6746 | 7161 | 7289 | 6552 | 1.03 |

## smallfiles (MiB/s)

| threads | tokio | tokio-chunked | rayon | compio | tokio/compio |
|---|---|---|---|---|---|
| 1 | 183 (0.9s cpu) | 421 (0.4s cpu) | 435 (0.3s cpu) | 125 (1.2s cpu) | 1.46 |
| 16 | 948 | 3602 | 3614 | 286 | 3.32 |

## write (MiB/s)

| threads | tokio | tokio-chunked | rayon | compio | tokio/compio |
|---|---|---|---|---|---|
| 1 | 3664 (1.1s cpu) | 3930 (1.0s cpu) | 3940 (1.0s cpu) | 3417 (0.9s cpu) | 1.07 |
| 16 | 8500 | 8946 | 7888 | 4427 | 1.92 |
