// $readmemh / $readmemb (IEEE 1800 21.4): addresses (@), comments, x/z digits, start and end
// addresses; $writememh writes back what was read. A [N:0] memory without a range gets the
// reference's warning on stdout.
module t;
  integer errors = 0;
  reg [7:0] m [0:7];
  reg [3:0] b [3:0];
  reg [7:0] m2 [0:7];
  integer i;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %h want %h", line, got[7:0], want[7:0]);
      errors = errors + 1;
    end
  endtask
  initial begin
    for (i = 0; i < 8; i++) m[i] = 8'h55;
    $readmemh("mem.hex", m);
    check(`__LINE__, m[0], 8'h0a);
    check(`__LINE__, m[1], 8'h0b);
    check(`__LINE__, m[2], 8'h0c);
    check(`__LINE__, m[3], 8'h55);        // not in the file: unchanged
    check(`__LINE__, m[6], 8'hff);
    check(`__LINE__, m[7], 8'h1x);
    $readmemb("mem.bin", b);              // no range: from the lowest address up (1364-2005)
    check(`__LINE__, b[0], 4'b0101);
    check(`__LINE__, b[1], 4'b1x0z);
    $readmemb("mem.bin", b, 3, 2);        // explicit start and end: downwards
    check(`__LINE__, b[3], 4'b0101);
    check(`__LINE__, b[2], 4'b1x0z);
    $writememh("copy.hex", m);
    $readmemh("copy.hex", m2);
    for (i = 0; i < 8; i++) check(`__LINE__, m2[i], m[i]);
    if (errors == 0) $display("PASSED");
  end
endmodule
