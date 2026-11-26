// equality of unpacked arrays (IEEE 1800-2017 7.4.3, 11.4.5): elements paired left to
// right; == gives x when no pair differs but one is unknown, === compares x bits
module top;
  logic [3:0] a [0:3], b [4:7], c [3:0];
  int m [2][3], n [2][3];
  int fails = 0;
  logic r;
  initial begin
    a = '{1, 2, 3, 4};
    b = '{1, 2, 3, 4};
    c = '{4, 3, 2, 1};
    if (!(a == b) || (a != b) || !(a === b)) fails++;
    if (a == c) fails++;
    // slices: a[0:1] and b[4:5] are {1, 2}, c[1:0] is {2, 1}
    if (a[0:1] != b[4:5]) fails++;
    if (a[0:1] == c[1:0]) fails++;
    b[6] = 4'bx;
    r = (a == b);
    if (r !== 1'bx) fails++;
    r = (a != b);
    if (r !== 1'bx) fails++;
    if (a === b) fails++;
    b[5] = 0;
    r = (a == b);
    if (r !== 1'b0) fails++;
    a[2] = 4'bx;
    b = a;
    if (!(a === b) || (a !== b)) fails++;
    // a multi-dimensional array, subarrays
    m = '{'{1, 2, 3}, '{4, 5, 6}};
    n = m;
    if (m != n) fails++;
    n[1][2] = 0;
    if (m == n || m[0] != n[0] || m[1] == n[1]) fails++;
    // elements are compared as before
    if (a[0] != 1 || m[1][1] != 5) fails++;
    if (fails == 0) $display("PASSED");
    else $display("FAILED %0d", fails);
  end
endmodule
