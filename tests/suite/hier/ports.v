// Ports (IEEE 1800 23.2-23.3): ANSI and non-ANSI headers, implicit nets, width mismatch
// (truncation, zero/sign extension), unconnected inputs (z) and outputs, .name and .*
// connections, inout ports, arrays of instances with sliced connections
module add(input [3:0] a, b, output [4:0] s);
  assign s = a + b;
endmodule
module old(a, y);
  input [3:0] a;
  output y;
  wire y = ^a;
endmodule
module sx(input signed [3:0] i, output signed [7:0] o);
  assign o = i;
endmodule
module bi(inout w, input en, input v);
  assign w = en ? v : 1'bz;
endmodule
module t;
  integer errors = 0;
  reg [3:0] a, b;
  wire [4:0] s;
  wire [2:0] short;
  wire y, w;
  wire [7:0] o;
  wire [4:0] unc;
  reg en1, en2;
  wire [3:0] q4;
  wire [15:0] busy;
  add u1(.a, .b, .s);                  // .name
  add u2(.*, .s(short));               // .* and a narrower output: truncated
  add u3(.a(a), .b(), .s(unc));        // an unconnected input is z: the sum is x
  old u4(a, y);
  sx u5(4'sb1010, o);                  // sign extension inside
  bi u6(w, en1, 1'b1);
  bi u7(w, en2, 1'b0);
  add arr[3:0] (.a(16'h1234 >> 0), .b(4'h1), .s());   // a[3:0] slices of 16 bits
  task check(input integer line, input [63:0] got, input [63:0] want);
    if (got !== want) begin
      $display("FAILED line %0d: got %b want %b", line, got[15:0], want[15:0]);
      errors = errors + 1;
    end
  endtask
  initial begin
    a = 4'd9; b = 4'd8; en1 = 1; en2 = 0;
    #1;
    check(`__LINE__, s, 17);
    check(`__LINE__, short, 3'b001);
    check(`__LINE__, unc, 5'bxxxxx);
    check(`__LINE__, y, ^4'd9);
    check(`__LINE__, o, 8'hfa);
    check(`__LINE__, w, 1);
    en1 = 0; en2 = 1;
    #1 check(`__LINE__, w, 0);
    en2 = 0;
    #1 check(`__LINE__, w, 1'bz);
    check(`__LINE__, arr[3].a, 4'h1);   // the leftmost instance gets the top slice
    check(`__LINE__, arr[0].a, 4'h4);
    if (errors == 0) $display("PASSED");
  end
endmodule
