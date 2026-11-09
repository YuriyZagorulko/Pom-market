import "./globals.css";
import { SessionProvider } from "@/lib/session";

export const metadata = { title: "Pom-Market Admin" };

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body>
        <SessionProvider role="admin">{children}</SessionProvider>
      </body>
    </html>
  );
}
