mmap tx ptr and rx ptr off by 8,388,608

rx/tx queue size = 4096
frame size = 2048
frame count = 8192

Thu Mar 18 12:32:58 AM EDT 2021
Got send and recv working in a hacky way. Haven't really found a clean way of creating separate umem regions for tx and rx. Resorted to copying the mmap region pointers to two umem structs.

I could either try and find a cleaner way of handling that, or try to make progress on actual scanner logic.

Check to see if it still works after swapping tx/rx umem/frames assignment back to original
Mon Mar 29 10:29:55 PM EDT 2021
