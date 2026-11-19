// Whole multi-dimensional arrays and subarrays (a multi-dimensional array with its first
// dimensions indexed) in assignments and as arguments (IEEE 1800-2017 7.4.5, 7.6):
// elements pair from the left, also between dimensions of opposite directions.
module t;
  integer errors = 0;
  int a[2][3][4], b[2][3][4];
  logic [7:0] d[1:0][0:2];             // a descending first dimension
  logic [7:0] u[0:2], r[2:0];
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0h want %0h", line, got, want);
      errors = errors + 1;
    end
  endtask
  function automatic int sum(input int v[4]);
    sum = 0;
    foreach (v[i]) sum += v[i];
  endfunction
  initial begin
    foreach (a[i, j, k]) a[i][j][k] = i * 100 + j * 10 + k;
    b[1][1] = a[0][2];                 // a subarray of 4 elements
    check(`__LINE__, {b[1][1][0], b[1][1][3]}, {32'd20, 32'd23});
    b[0] = a[1];                       // of 3 x 4 elements
    check(`__LINE__, {b[0][0][0], b[0][2][3]}, {32'd100, 32'd123});
    b = a;                             // the whole array
    check(`__LINE__, {b[1][2][1], b[0][1][2]}, {32'd121, 32'd12});
    check(`__LINE__, sum(a[1][2]), 120 + 121 + 122 + 123);
    u[0] = 8'h10; u[1] = 8'h11; u[2] = 8'h12;
    d[1] = u;                          // d[1][0..2] = u[0..2]
    check(`__LINE__, {d[1][0], d[1][2]}, 16'h1012);
    r = d[1];                          // r[2] = d[1][0]: left to right
    check(`__LINE__, {r[2], r[1], r[0]}, 24'h101112);
    d[0] <= d[1];
    #1 check(`__LINE__, d[0][1], 8'h11);
    if (errors == 0) $display("PASSED");
  end
endmodule
