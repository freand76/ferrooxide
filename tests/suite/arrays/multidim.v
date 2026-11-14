// Arrays (IEEE 1800 7.4): multi-dimensional unpacked arrays, packed multi-dimensional
// vectors, element and part selects through several dimensions, run-time indexes, arrays
// of reals, writes and reads out of range
module t;
  integer errors = 0;
  reg [7:0] m [0:2][0:3];
  reg [3:0][7:0] p;                  // 4 bytes packed
  reg [1:0][3:0] pm [0:1];           // an array of packed 2x4
  real ra [0:2];
  integer i, j;
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %0h want %0h", line, got, want);
      errors = errors + 1;
    end
  endtask
  initial begin
    for (i = 0; i < 3; i++)
      for (j = 0; j < 4; j++)
        m[i][j] = i * 16 + j;
    check(`__LINE__, m[2][3], 8'h23);
    check(`__LINE__, m[1][0][4], 1);         // a bit of an element
    i = 2; j = 1;
    check(`__LINE__, m[i][j], 8'h21);
    check(`__LINE__, m[i][j][7:4], 4'h2);
    m[i][j][3:0] = 4'hf;
    check(`__LINE__, m[2][1], 8'h2f);
    check(`__LINE__, m[3][0], 8'hxx);        // out of range: x
    m[3][0] = 0;                             // and not written
    check(`__LINE__, $size(m, 1), 3);
    check(`__LINE__, $size(m, 2), 4);
    p = 32'h44332211;
    check(`__LINE__, p[0], 8'h11);           // the rightmost dimension is the byte
    check(`__LINE__, p[3], 8'h44);
    check(`__LINE__, p[2][7:4], 4'h3);
    p[1] = 8'hab;
    check(`__LINE__, p, 32'h4433ab11);
    check(`__LINE__, $bits(p), 32);
    pm[1] = 8'h5a;
    check(`__LINE__, pm[1][1], 4'h5);
    check(`__LINE__, pm[1][0], 4'ha);
    pm[0][1] = 4'h7;
    check(`__LINE__, pm[0][1][2], 1);
    check(`__LINE__, pm[1][1:0], 8'h5a);       // a range over the element's packed dimension
    i = 1; j = 0;
    pm[i][j][2:1] = 2'b11;                     // a part inside it, run-time indexes
    check(`__LINE__, pm[1], 8'h5e);
    check(`__LINE__, pm[1][0][3:1], 3'b111);
    ra[1] = 2.5;
    if (ra[1] != 2.5) begin $display("FAILED line %0d", `__LINE__); errors++; end
    if (errors == 0) $display("PASSED");
  end
endmodule
