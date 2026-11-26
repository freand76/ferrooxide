// bit and part selects of a struct member that is a packed struct (IEEE 1800-2017 7.2.1: a
// packed struct is a vector), as targets and in expressions
module top;
  typedef struct packed { logic a, b, c, d; } f_s;
  typedef struct packed { logic [3:0] x; f_s flags; } m_s;
  m_s m;
  initial begin
    m = '0;
    for (int i = 0; i < 4; i++) m.flags[i] = i[0];
    m.flags[3:2] = 2'b01;
    m.x[1] = 1;
    if (m === 8'b0010_0110 && m.flags[0] === 1'b0 && m.flags[2:1] === 2'b11 && m.flags.b === 1'b1) $display("PASSED");
    else $display("FAILED %b", m);
  end
endmodule
