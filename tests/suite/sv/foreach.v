// foreach (IEEE 1800 12.7.3): loop variables run from the left to the right bound of their
// dimension; unpacked dimensions first, then packed; empty positions skip a dimension
module t;
  integer errors = 0, n;
  reg [7:0] m [0:3];
  reg [0:2] v;
  reg [3:0] w [1:2][3:1];
  int a [5];
  reg [255:0] order;
  task check(input integer line, input [255:0] got, input [255:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0h want %0h", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    foreach (m[i]) m[i] = i * 3;
    check(`__LINE__, {m[0], m[1], m[2], m[3]}, {8'd0, 8'd3, 8'd6, 8'd9});
    order = 0;
    foreach (v[i]) order = order * 10 + i;
    check(`__LINE__, order, 12);                  // 0, 1, 2
    order = 0; n = 0;
    foreach (w[i, j]) begin order = order * 100 + i * 10 + j; n++; end
    check(`__LINE__, n, 6);
    check(`__LINE__, order, 'd131211232221);      // [1:2] up, [3:1] down
    order = 0;
    foreach (w[, j]) order = order * 10 + j;
    check(`__LINE__, order, 321);
    n = 0;
    foreach (m[i, b]) n += m[i][b];               // the packed dimension of the elements
    check(`__LINE__, n, 2 + 2 + 2);               // 0, 3 (2 ones), 6 (2), 9 (2)
    n = 0;
    foreach (m[]) n++;                            // no loop variables: no iterations
    check(`__LINE__, n, 0);
    foreach (a[i]) a[i] = 4 - i;                  // int a[5]: [0:4]
    check(`__LINE__, a[0] * 10 + a[4], 40);
    if (errors == 0) $display("PASSED");
  end
endmodule
